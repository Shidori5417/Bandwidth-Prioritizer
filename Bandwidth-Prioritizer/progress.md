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
- [x] Windows: ETW (Event Tracing for Windows) `Microsoft-Windows-TCPIP` entegrasyonu (ferrisetw)
- [x] Windows: `GetExtendedTcpTable` polling fallback mekanizması
- [ ] PID → Process Name eşleştirme
- [x] Aktif ağ bağlantılarını MonitorEvent akışıyla (stream) bildirme
- [x] Polling döngüsü (graceful fallback)
- [ ] Kritik uygulama listesi ile eşleşme kontrolü
- [ ] **Test:** Mock süreç listesiyle unit testler

---

### 🚧 FAZ 2 — Konfigürasyon Motoru (Config Manager)

**Hedef:** Kullanıcı kurallarını yönet.

- [x] `config.toml` şeması tasarımı
- [x] `serde` ile deserializasyon
- [x] Kritik uygulama listesi (whitelist)
- [x] Throttle hedef değerleri (Kbps / Mbps)
- [x] Profil desteği (Gaming, Meeting, Download)
- [x] Hot-reload (dosya değişikliğini izle, restart gerektirmez)
- [x] **Test:** Geçersiz config senaryoları

---

### 🚧 FAZ 3 — Trafik Kontrolcüsü (Traffic Controller)

**Hedef:** Gerçek bant genişliği kısıtlamasını uygula.

#### Linux

- [ ] `libbpf-rs` ile eBPF entegrasyonu araştırması
- [x] `tc` (traffic control) komutu ile `htb` qdisc programatik yönetimi
- [x] Alternatif: `nftables` kuralları ile hız sınırlama

#### Windows

- [ ] Windows Filtering Platform (WFP) API entegrasyonu
- [x] Windows: `windivert` ile paket arayüzüne (flow-level filter ile) entegrasyon
- [x] Alternatif: `NetLimiter` benzeri yaklaşım (socket-level)

#### Ortak

- [x] Token Bucket algoritması implementasyonu (hız sınırlayıcı çekirdeği)
- [x] Per-PID kural ekleme/kaldırma API'si
- [ ] **Test:** Sanal ağ arayüzü ile entegrasyon testleri

---

### 🚧 FAZ 4 — Throttle Motoru (Throttle Engine)

**Hedef:** Dinamik kısıtlama mantığını işlet.

- [x] State machine: `Normal` → `PriorityMode` → `Normal` (bwp-engine crate)
- [x] Kritik uygulama başlatıldığında otomatik tetikleme
- [x] Kritik uygulama kapandığında geri alma (graceful restore)
- [ ] Kademeli kısıtlama (ani kesim yerine yumuşak geçiş)
- [x] CPU/bellek kullanımı gözetimi (lightweight kalması için)
- [ ] **Test:** Senaryo tabanlı durum geçiş testleri

---

### 🚧 FAZ 5 — CLI / Yönetim Katmanı (Entegrasyon)

**Hedef:** CLI ve arka plan daemon (engine) iletişimini sağla.

- [x] Engine içinde IPC Server (Named Pipes / Unix Sockets) & Event birleştirme
- [ ] `bwp-cli` inşası
- [ ] CLI'den IPC komutları gönderme
  - `bwp status`
  - `bwp set-profile <name>`
- [ ] `ratatui` ile gerçek zamanlı TUI ve anlık durum ekranı
- [ ] **Test:** End-to-end (E2E) entegrasyon testleri

---

### 🔲 FAZ 6 — Daemon & Sistem Entegrasyonu

**Hedef:** Arka planda çalışan sistem servisi.

- [ ] Linux: `systemd` unit dosyası
- [ ] Windows: Windows Service entegrasyonu
- [ ] Ayrıcalık yönetimi (root/admin gerektiren işlemler için güvenli yükseltme)
- [ ] IPC mekanizması (CLI ↔ Daemon iletişimi için Unix socket / named pipe)
- [ ] Graceful shutdown & sinyal yönetimi (`SIGTERM`, `SIGINT`)

---

### 🔲 FAZ 7 — Test, Optimizasyon & Dağıtım

- [ ] End-to-end test senaryoları
- [ ] Performans profili (CPU < %1, RAM < 20MB hedef)
- [ ] Paketleme: `.deb`, `.rpm`, Windows installer
- [ ] Dokümantasyon (README, man page, wiki)
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

| Faz                           | Durum         | Tamamlanma |
| ----------------------------- | ------------- | ---------- |
| FAZ 0 — Kurulum               | ✅ Tamamlandı | 100%       |
| FAZ 1 — Süreç İzleyici        | 🔲 Başlamadı  | 0%         |
| FAZ 2 — Config Motoru         | 🔲 Başlamadı  | 0%         |
| FAZ 3 — Trafik Kontrolcüsü    | 🔲 Başlamadı  | 0%         |
| FAZ 4 — Throttle Motoru       | 🔲 Başlamadı  | 0%         |
| FAZ 5 — CLI / Yönetim Katmanı | 🔲 Başlamadı  | 0%         |
| FAZ 6 — Daemon                | 🔲 Başlamadı  | 0%         |
| FAZ 7 — Test & Dağıtım        | 🔲 Başlamadı  | 0%         |

---

## Notlar & Kararlar

- **Dil:** Rust (güvenli bellek yönetimi, sistem düzeyinde erişim, sıfır overhead)
- **Platform önceliği:** Linux (eBPF sayesinde kernel düzeyinde güç), ardından Windows
- **Throttle yöntemi:** Kernel-space çözüm tercih edilecek; user-space fallback hazır tutulacak
- **Lisans:** MIT / Apache 2.0 (çift lisans)
