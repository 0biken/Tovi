import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import "./index.css";
import Layout from "./components/Layout";
import DeviceListPage from "./pages/DeviceListPage";
import SendPage from "./pages/SendPage";
import QrPairingPage from "./pages/QrPairingPage";
import HistoryPage from "./pages/HistoryPage";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <BrowserRouter>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<DeviceListPage />} />
          <Route path="send/:deviceId" element={<SendPage />} />
          <Route path="pair" element={<QrPairingPage />} />
          <Route path="history" element={<HistoryPage />} />
        </Route>
      </Routes>
    </BrowserRouter>
  </React.StrictMode>
);
