//! LAN device discovery (mDNS / DNS-SD, `_tovi._udp.local`).
//!
//! Discovery is platform-pluggable: desktop uses `mdns-sd` directly, while
//! Android (NSD) and iOS (Bonjour / Network framework) supply their own adapters.
