package app.tovi.android.ui

import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.mlkit.vision.MlKitAnalyzer
import androidx.camera.view.CameraController
import androidx.camera.view.LifecycleCameraController
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode

/**
 * Back-camera preview that reports TOVI pairing links found in QR codes.
 * [onLink] is called on the main thread for every frame that has one, so the
 * caller should ignore repeats. Needs the CAMERA permission.
 */
@Composable
fun QrScanner(onLink: (String) -> Unit, modifier: Modifier = Modifier) {
  val context = LocalContext.current
  val lifecycleOwner = LocalLifecycleOwner.current
  val currentOnLink by rememberUpdatedState(onLink)
  val controller =
    remember {
      LifecycleCameraController(context).apply {
        cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA
        setEnabledUseCases(CameraController.IMAGE_ANALYSIS)
      }
    }

  DisposableEffect(lifecycleOwner) {
    val scanner =
      BarcodeScanning.getClient(BarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build())
    val mainThread = ContextCompat.getMainExecutor(context)
    controller.setImageAnalysisAnalyzer(
      mainThread,
      // Positions aren't used, so the original image's coordinates will do
      MlKitAnalyzer(listOf(scanner), ImageAnalysis.COORDINATE_SYSTEM_ORIGINAL, mainThread) { result ->
        result.getValue(scanner)?.firstNotNullOfOrNull { PairingLink.parse(it.rawValue) }?.let(currentOnLink)
      },
    )
    controller.bindToLifecycle(lifecycleOwner)
    onDispose {
      controller.unbind()
      controller.clearImageAnalysisAnalyzer()
      scanner.close()
    }
  }

  AndroidView(
    factory = { PreviewView(it).apply { this.controller = controller } },
    modifier = modifier,
  )
}
