# Bandwidth-Prioritizer — Progress Tracker

## Proje Özeti

Kritik uygulamalar (Discord, Zoom, VS Code Live Share vb.) aktifken diğer tüm uygulamaların internet bant genişliğini **dinamik olarak** kısıtlayan, Rust ile yazılmış sistem düzeyinde bir ağ trafiği şekillendirici.

---

## Mimari Genel Bakış

```
┌─────────────────────────────────────────────────┐
│                Bandwidth-Prioritizer            │
│                                                 │
│  ┌─────────────┐    ┌──────────────────────┐   │
│  │ Process     │───▶│  Traffic Controller  │   │
│  │ Monitor     │    │  (WFP / eBPF / tc)   │   │
│  └─────────────┘    └──────────┬───────────┘   │
│         │                      │               │
│  ┌─────────────┐    ┌──────────▼───────────┐   │
│  │  Config     │    │   Throttle Engine    │   │
│  │  Manager    │    │   (Token Bucket)     │   │
│  └─────────────┘    └──────────────────────┘   │
│         │                                       │
│  ┌─────────────┐                               │
│  │  CLI / TUI  │                               │
│  └─────────────┘                               │
└─────────────────────────────────────────────────┘
```

---

## Faz Planı

### ✅ FAZ 0 — Proje Kurulumu

- [x] Rust workspace yapısı (`Cargo.toml`)
- [x] Crate mimarisi belirleme (`core`, `monitor`, `controller`, `cli`)
- [x] CI/CD pipeline (GitHub Actions)
- [x] Linting & formatting (`clippy`, `rustfmt`)
- [x] Temel dokümantasyon iskeleti

---

### 🚧 FAZ 1 — Süreç İzleyici (Process Monitor)

**Hedef:** Sistemde çalışan süreçleri ve ağ bağlantılarını tespit et.

- [ ] `/proc/net/tcp` & `/proc/net/udp` okuma (Linux)
- [ ] Windows: ETW (Event Tracing for Windows) `Microsoft-Windows-TCPIP` entegrasyonu (ferrisetw) — henüz uygulanmadı; iskelet kaldırıldı, `spawn_etw_listener` hata döner
- [x] Windows: `GetExtendedTcpTable` / `GetExtendedUdpTable` polling (IPv4 + IPv6, ERROR_INSUFFICIENT_BUFFER yeniden deneme döngüsü, ağ bayt sırası dönüşümü) ve ardışık snapshot'ların `ConnectionOpened`/`ConnectionClosed` olaylarına diff'lenmesi. Diff/parse mantığı platformdan bağımsız ve Linux'ta test edilir; FFI kısmı yalnızca Windows CI'da derlenir, gerçek makinede çalıştırılmadı. Dinleme (LISTEN) ve loopback soketleri yok sayılır
- [x] PID → Process Name eşleştirme (DashMap tabanlı cache)
- [x] Aktif ağ bağlantılarını MonitorEvent akışıyla (stream) bildirme
- [x] Polling döngüsü (`[general] poll_interval_ms` ile ayarlanır, varsayılan 500 ms)
- [x] Kritik uygulama listesi ile eşleşme kontrolü
- [x] **Test:** Mock süreç listesiyle unit testler (bwp-monitor)

---

### 🚧 FAZ 2 — Konfigürasyon Motoru (Config Manager)

**Hedef:** Kullanıcı kurallarını yönet.

- [x] `config.toml` şeması tasarımı
- [x] `serde` ile deserializasyon
- [x] Kritik uygulama listesi (whitelist)
- [x] Throttle hedef değerleri (Kbps / Mbps)
- [x] Profil desteği (Gaming, Meeting, Download)
- [x] Hot-reload (dosya değişikliğini izle, restart gerektirmez)
- [x] Reload sonrası motor, izlenen tüm PID'leri yeni `critical_apps`/profil/limit'e göre yeniden değerlendirir (Priority Mode giriş/çıkış, limit güncelleme, silinen profilin temizlenmesi) — birim testli
- [x] `[shaping] experimental_windivert` anahtarı (varsayılan `false`, yalnızca başlangıçta okunur)
- [x] **Test:** Geçersiz config senaryoları

---

### 🚧 FAZ 3 — Trafik Kontrolcüsü (Traffic Controller)

**Hedef:** Gerçek bant genişliği kısıtlamasını uygula.

#### Linux

- [ ] `libbpf-rs` ile eBPF entegrasyonu araştırması
- [ ] `tc` (traffic control) ile `htb` qdisc yönetimi — uygulanmadı; Linux controller no-op. (Motorun kendisi de yalnızca Windows'ta çalışır, Linux'ta bağlantı izleyici yok, bu yüzden bilinçli olarak yapılmadı)
- [ ] Alternatif: `nftables` kuralları ile hız sınırlama — uygulanmadı

#### Windows

- [ ] Windows Filtering Platform (WFP) API entegrasyonu
- [~] Windows: WinDivert NETWORK katmanı ile paket şekillendirme — **deneysel, opt-in, varsayılan KAPALI, gerçek Windows'ta test edilmedi.** Cargo feature `experimental-windivert` + `[shaping] experimental_windivert = true` gerekir. Paketler yerel port -> PID tablosu ile eşlenir; kısıtlanmayan/bilinmeyen her paket anında geri enjekte edilir, kısıtlananlar PID başına token bucket ile kuyrukta geciktirilir (500 ms üstü düşürülür). Hata/kapanışta handle önce `shutdown(RECV)` edilip kapatılır. Varsayılan derleme hiçbir zaman WinDivert handle'ı açmaz
- [ ] Alternatif: `NetLimiter` benzeri yaklaşım (socket-level) — uygulanmadı

#### Ortak

- [x] Token Bucket algoritması (borçlanmalı `reserve_at` ile gecikme hesabı) ve gecikme kuyruklu `Shaper` — platformdan bağımsız, birim testli
- [x] Per-PID kural ekleme/kaldırma API'si
- [ ] **Test:** Sanal ağ arayüzü ve WinDivert ile entegrasyon testleri (bwp-controller) — yapılmadı; WinDivert yolu gerçek donanımda hiç çalıştırılmadı

---

### 🚧 FAZ 4 — Throttle Motoru (Throttle Engine)

**Hedef:** Dinamik kısıtlama mantığını işlet.

- [x] State machine: `Normal` → `PriorityMode` → `Normal` (bwp-engine crate)
- [x] Kritik uygulama başlatıldığında otomatik tetikleme
- [x] Kritik uygulama kapandığında geri alma (graceful restore)
- [x] Kademeli kısıtlama (Smooth transition logic)
- [x] CPU/bellek kullanımı gözetimi (Lightweight engine design)
- [x] **Test:** Senaryo tabanlı durum geçiş testleri (birim testleri, sahte controller ile)

---

### 🚧 FAZ 5 — CLI / Yönetim Katmanı (Entegrasyon)

**Hedef:** CLI ve arka plan daemon (engine) iletişimini sağla.

- [x] Engine içinde IPC Server (Named Pipes / Unix Sockets) & Event birleştirme
- [x] `bwp-cli` inşası ve Daemon lifecycle (Start/Stop) entegrasyonu
- [x] CLI'den IPC komutları gönderme (`status`, `set`, `add`, vb.)
- [x] `ratatui` ile gerçek zamanlı TUI (`top` komutu) ve anlık durum ekranı
- [x] V2 için not: `StatusPayload` üzerinde gelen `recent_events` ile ETW log akışını TUI'da gösterme (Ertelendi)
- [ ] **Test:** End-to-end (E2E) entegrasyon testleri — yok (yalnızca birim testleri)

---

### 🚧 FAZ 6 — Daemon & Sistem Entegrasyonu

**Hedef:** Arka planda çalışan sistem servisi.

- [ ] Linux: `systemd` unit dosyası
- [x] Windows: Windows Service entegrasyonu (`windows-service` crate)
- [ ] Ayrıcalık yönetimi (SYSTEM/LocalService context)
- [x] Graceful shutdown & sinyal yönetimi (`SIGTERM`, `SIGINT`, SCM Stop)
- [x] Install/Uninstall komutları (`bwp service`)

---

### 🚧 FAZ 7 — Test, Optimizasyon & Dağıtım

- [ ] End-to-end test senaryoları (gerçek ağ/Windows üzerinde) — yapılmadı
- [ ] Performans profili (CPU < %1, RAM < 20MB hedef) — ölçülmedi
- [x] Paketleme: PowerShell Release Script (`-WithWinDivert` ile deneysel şekillendiricili zip)
- [x] Dokümantasyon: README (man page ve wiki yok)
- [ ] v1.0.0 release

---

## Bağımlılıklar (Planlanan Crates)

| Crate                  | Amaç                       |
| ---------------------- | -------------------------- |
| `tokio`                | Async runtime              |
| `serde` / `serde_toml` | Konfigürasyon yönetimi     |
| `clap`                 | CLI argüman ayrıştırma     |
| `ratatui`              | Terminal kullanıcı arayüzü |
| `libbpf-rs`            | Linux eBPF entegrasyonu    |
| `windows-rs`           | Windows WFP API            |
| `tracing`              | Yapılandırılmış loglama    |
| `notify`               | Dosya izleme (hot-reload)  |
| `thiserror`            | Hata yönetimi              |

---

## Güncel Durum

| Faz                           | Durum                | Not |
| ----------------------------- | -------------------- | --- |
| FAZ 0 — Kurulum               | ✅ Tamamlandı        | |
| FAZ 1 — Süreç İzleyici        | 🚧 Büyük kısmı tamam | Windows polling çalışıyor (gerçek makinede denenmedi); ETW ve Linux `/proc/net` yok |
| FAZ 2 — Config Motoru         | ✅ Tamamlandı        | Reload sonrası yeniden değerlendirme dahil |
| FAZ 3 — Trafik Kontrolcüsü    | 🚧 Deneysel          | WinDivert opt-in ve test edilmedi; Linux `tc` yok; entegrasyon testi yok |
| FAZ 4 — Throttle Motoru       | ✅ Tamamlandı        | Durum makinesi birim testli; gerçek kısıtlama FAZ 3'e bağlı |
| FAZ 5 — CLI / Yönetim Katmanı | ✅ Tamamlandı        | |
| FAZ 6 — Daemon                | 🚧 Kısmen            | Windows Service var; systemd unit ve ayrıcalık yönetimi yok |
| FAZ 7 — Test & Dağıtım        | 🚧 Kısmen            | Windows CI derliyor/test ediyor; gerçek ağ üzerinde E2E test yok; v1.0.0 yayınlanmadı |

### Kalanlar

- Gerçek Windows'ta polling izleyicisinin ve WinDivert şekillendiricinin doğrulanması
- ETW tabanlı olay akışı (polling yerine)
- Linux: `tc htb` controller ve bağlantı izleyici
- WFP / `NetLimiter` benzeri alternatifler
- Gerçek ağ ile E2E testler, systemd unit

---

## Notlar & Kararlar

- **Dil:** Rust (güvenli bellek yönetimi, sistem düzeyinde erişim, sıfır overhead)
- **Platform önceliği:** Fiilen Windows (motor yalnızca Windows'ta çalışır); Linux derlemesi geliştirme/CI içindir
- **Throttle yöntemi:** Kernel-space çözüm tercih edilecek; user-space fallback hazır tutulacak
- **Lisans:** MIT / Apache 2.0 (çift lisans)
