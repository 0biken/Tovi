import javax.inject.Inject

plugins {
  alias(libs.plugins.android.application)
  alias(libs.plugins.compose.compiler)
  alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "app.tovi.android"
    compileSdk = 36
    ndkVersion = "29.0.14206865"
    defaultConfig {
        applicationId = "app.tovi.android"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        // Only the ABIs libtovi_ffi.so is built for (rustAbis below); without
        // this, JNA's 32-bit libraries would let the app install where it can't run
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures {
      compose = true
      aidl = false
      buildConfig = false
      shaders = false
    }

    packaging {
      resources {
        excludes += "/META-INF/{AL2.0,LGPL2.1}"
      }
    }
}

kotlin {
    jvmToolchain(17)
}

// ------------------------------------------------------------------ Rust core
//
// libtovi_ffi.so is built from core/tovi-ffi with cargo-ndk, and the Kotlin
// bindings (package app.tovi.core) are generated from it with uniffi-bindgen.
// Needs: rustup targets aarch64-linux-android and x86_64-linux-android, and
// `cargo install cargo-ndk`. See apps/android/README.md.

/** Workspace root: apps/android/app → ../../.. */
val repoRoot = layout.projectDirectory.dir("../../..")
val rustAbis = listOf("arm64-v8a", "x86_64")

abstract class CargoNdkBuild @Inject constructor(private val exec: ExecOperations) : DefaultTask() {
  @get:InputFiles
  @get:PathSensitive(PathSensitivity.RELATIVE)
  abstract val sources: ConfigurableFileCollection

  @get:Internal abstract val workspace: DirectoryProperty

  @get:Internal abstract val ndkDir: DirectoryProperty

  @get:Input abstract val abis: ListProperty<String>

  @get:Input abstract val minSdk: Property<Int>

  @get:OutputDirectory abstract val outputDir: DirectoryProperty

  @TaskAction
  fun build() {
    val out = outputDir.get().asFile
    out.deleteRecursively()
    exec.exec {
      workingDir = workspace.get().asFile
      environment("ANDROID_NDK_HOME", ndkDir.get().asFile.absolutePath)
      commandLine(
        listOf("cargo", "ndk") +
          abis.get().flatMap { listOf("-t", it) } +
          listOf("-P", minSdk.get().toString(), "-o", out.absolutePath) +
          listOf("build", "-p", "tovi-ffi", "--release")
      )
    }
  }
}

abstract class UniffiBindgen @Inject constructor(private val exec: ExecOperations) : DefaultTask() {
  /** Any one ABI's library: the bindings are the same for all */
  @get:InputFile
  @get:PathSensitive(PathSensitivity.NONE)
  abstract val library: RegularFileProperty

  @get:InputFile
  @get:PathSensitive(PathSensitivity.NONE)
  abstract val config: RegularFileProperty

  @get:Internal abstract val workspace: DirectoryProperty

  @get:OutputDirectory abstract val outputDir: DirectoryProperty

  @TaskAction
  fun generate() {
    val out = outputDir.get().asFile
    out.deleteRecursively()
    exec.exec {
      workingDir = workspace.get().asFile
      commandLine(
        "cargo", "run", "-q", "-p", "tovi-ffi", "--features", "cli", "--bin", "uniffi-bindgen", "--",
        "generate", "--library", library.get().asFile.absolutePath,
        "--language", "kotlin", "--out-dir", out.absolutePath, "--no-format",
      )
    }
  }
}

val cargoBuild =
  tasks.register<CargoNdkBuild>("cargoNdkBuild") {
    description = "Builds libtovi_ffi.so for each ABI"
    sources.from(
      repoRoot.file("Cargo.toml"),
      repoRoot.file("Cargo.lock"),
      repoRoot.dir("core").asFileTree.matching { exclude("**/target/**") },
    )
    workspace.set(repoRoot)
    ndkDir.set(androidComponents.sdkComponents.ndkDirectory)
    abis.set(rustAbis)
    minSdk.set(26)
    outputDir.set(layout.buildDirectory.dir("rust/jniLibs"))
  }

val uniffiBindgen =
  tasks.register<UniffiBindgen>("uniffiBindgen") {
    description = "Generates the Kotlin bindings for tovi-ffi"
    library.set(cargoBuild.flatMap { it.outputDir.file("${rustAbis.first()}/libtovi_ffi.so") })
    config.set(repoRoot.file("core/tovi-ffi/uniffi.toml"))
    workspace.set(repoRoot)
    outputDir.set(layout.buildDirectory.dir("generated/uniffi/kotlin"))
  }

androidComponents {
  onVariants { variant ->
    variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoBuild, CargoNdkBuild::outputDir)
    variant.sources.kotlin?.addGeneratedSourceDirectory(uniffiBindgen, UniffiBindgen::outputDir)
  }
}

dependencies {
  val composeBom = platform(libs.androidx.compose.bom)
  implementation(composeBom)
  androidTestImplementation(composeBom)

  // Core Android dependencies
  implementation(libs.androidx.core.ktx)
  implementation(libs.androidx.lifecycle.runtime.ktx)
  implementation(libs.androidx.activity.compose)

  // Arch Components
  implementation(libs.androidx.lifecycle.runtime.compose)
  implementation(libs.androidx.lifecycle.viewmodel.compose)

  // Compose
  implementation(libs.androidx.compose.ui)
  implementation(libs.androidx.compose.ui.tooling.preview)
  implementation(libs.androidx.compose.material3)
  implementation(libs.androidx.compose.material.icons.core)
  // Tooling
  debugImplementation(libs.androidx.compose.ui.tooling)
  // Instrumented tests
  androidTestImplementation(libs.androidx.compose.ui.test.junit4)
  debugImplementation(libs.androidx.compose.ui.test.manifest)

  // Rust core bindings: UniFFI's Kotlin code calls the library through JNA
  // and its async functions are coroutines
  implementation(libs.jna) { artifact { type = "aar" } }
  implementation(libs.kotlinx.coroutines.android)

  // Local tests: jUnit, coroutines, Android runner
  testImplementation(libs.junit)
  testImplementation(libs.kotlinx.coroutines.test)

  // Instrumented tests: jUnit rules and runners
  androidTestImplementation(libs.androidx.test.core)
  androidTestImplementation(libs.androidx.test.ext.junit)
  androidTestImplementation(libs.androidx.test.runner)
  androidTestImplementation(libs.androidx.test.espresso.core)

  // Navigation
  implementation(libs.androidx.navigation3.ui)
  implementation(libs.androidx.navigation3.runtime)
  implementation(libs.androidx.lifecycle.viewmodel.navigation3)
}
