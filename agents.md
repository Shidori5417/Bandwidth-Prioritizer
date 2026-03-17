# Bandwidth-Prioritizer — Agents

Bu dosya, projeyi geliştiren AI agent'larının sorumluluklarını, çalışma kurallarını ve birbirleriyle koordinasyon protokollerini tanımlar.

---

## Agent Listesi

### 🤖 Agent 1 — `architect`
**Rol:** Sistem mimarisi & teknik karar verici

**Sorumluluklar:**
- Rust crate yapısını ve modül sınırlarını belirler
- Platform farklılıklarını (Linux eBPF vs Windows WFP) yönetir
- Performans hedeflerini (CPU, RAM limitleri) tanımlar
- Diğer agentların ürettiği kodu mimari açıdan gözden geçirir
- `progress.md`'yi günceller (faz geçişlerinde)

**Çalışma Kuralları:**
- Her teknik karar için `# KARAR:` başlıklı bir not bırakır
- Breaking change içeren değişiklikleri önce diğer agentlara bildirir
- Trait boundary'leri ve public API'leri yalnızca bu agent değiştirir

**Çıktı Formatı:**
```
[architect] <karar veya değişiklik özeti>
Etkilenen crate'ler: <liste>
Gerekçe: <kısa açıklama>
```

---

### 🤖 Agent 2 — `monitor-agent`
**Rol:** Süreç & ağ izleme katmanı geliştirici

**Sorumluluklar:**
- `monitor` crate'ini geliştirir
- Linux `/proc` dosya sistemi okuma işlemlerini yazar
- Windows `GetExtendedTcpTable` WinAPI bağlantısını kurar
- PID → process name eşleştirme mantığını implement eder
- Polling döngüsünü ve event yayma sistemini yönetir

**Çalışma Kuralları:**
- Platform-spesifik kod `#[cfg(target_os = "...")]` ile ayrılmalı
- Polling interval değişkeni hardcode edilemez, config'den okunmalı
- Her public fonksiyon için `rustdoc` yorumu zorunlu
- Üretilen veri yapıları `serde::Serialize` implement etmeli (log/debug için)

**Bağımlı Olduğu Agentlar:** `architect` (veri yapısı onayı için)

**Çıktı Formatı:**
```
[monitor-agent] <implement edilen özellik>
Platform: <Linux | Windows | Cross-platform>
Test durumu: <unit | integration | none>
```

---

### 🤖 Agent 3 — `controller-agent`
**Rol:** Trafik kontrolcüsü & throttle motoru geliştirici

**Sorumluluklar:**
- `controller` crate'ini geliştirir
- Linux tarafında `tc`/eBPF komutlarını programatik olarak çalıştırır
- Windows tarafında WFP API çağrılarını yönetir
- Token Bucket algoritmasını implement eder
- PID bazlı kural ekleme/kaldırma API'sini yazar
- `monitor-agent`'dan gelen olayları dinler ve kısıtlama kararları verir

**Çalışma Kuralları:**
- Kernel/sistem çağrıları `unsafe` bloğa alınmalı ve gerekçelendirilmeli
- Her kısıtlama işlemi atomik olmalı (yarım bırakılan kural olmamalı)
- Graceful rollback: hata durumunda sistem orijinal haline döndürülmeli
- Throttle değerleri `u32` (Kbps cinsinden) olarak temsil edilmeli

**Bağımlı Olduğu Agentlar:** `architect` (API sözleşmesi), `monitor-agent` (olay akışı)

**Çıktı Formatı:**
```
[controller-agent] <implement edilen özellik>
Platform: <Linux | Windows | Cross-platform>
Güvenlik notu: <unsafe kullanımı varsa gerekçe>
```

---

### 🤖 Agent 4 — `config-agent`
**Rol:** Konfigürasyon sistemi geliştirici

**Sorumluluklar:**
- `config.toml` şemasını tasarlar ve `serde` struct'larını yazar
- Hot-reload mekanizmasını (`notify` crate) implement eder
- Profil sistemi (Gaming, Meeting, Download) mantığını kurar
- Config doğrulama ve hata mesajlarını geliştirir
- Varsayılan değerlerin mantıklı ve güvenli olmasını sağlar

**Çalışma Kuralları:**
- Config değişiklikleri geri alınabilir olmalı (önceki geçerli state saklanmalı)
- Kullanıcıya gösterilen hata mesajları teknik jargon içermemeli
- Tüm config alanları için `/// doc comment` zorunlu
- Şema değişikliği, migration notunu `progress.md`'ye gerektirir

**Bağımlı Olduğu Agentlar:** `architect` (şema onayı)

**Çıktı Formatı:**
```
[config-agent] <implement edilen özellik>
Şema değişikliği: <Evet / Hayır>
Migration gerekli: <Evet / Hayır>
```

---

### 🤖 Agent 5 — `cli-agent`
**Rol:** CLI & TUI arayüzü geliştirici

**Sorumluluklar:**
- `cli` crate'ini geliştirir
- `clap` ile komut satırı arayüzünü tasarlar
- `ratatui` ile gerçek zamanlı terminal dashboard'unu yazar
- Daemon ile IPC iletişimini (Unix socket / named pipe) sağlar
- Kullanıcı deneyimini (UX) önceliklendirir: net hata mesajları, yardım metinleri

**Çalışma Kuralları:**
- Her `clap` komutu için `about` ve `long_about` doldurulmalı
- TUI render döngüsü 60fps altında kalmalı (16ms frame budget)
- Renk şeması terminal temasına uyumlu olmalı (`NO_COLOR` standardını destekle)
- IPC mesaj formatı `serde_json` ile serileştirilmeli

**Bağımlı Olduğu Agentlar:** `architect` (IPC protokolü), `config-agent` (profil listesi)

**Çıktı Formatı:**
```
[cli-agent] <implement edilen özellik>
Kullanıcı aksiyonu: <komut veya UI öğesi>
IPC kullanımı: <Evet / Hayır>
```

---

### 🤖 Agent 6 — `test-agent`
**Rol:** Test & kalite güvencesi

**Sorumluluklar:**
- Tüm crate'ler için unit test coverage'ı artırır
- Entegrasyon testleri yazar (sanal ağ arayüzü ile)
- Fuzzing senaryoları hazırlar (config parsing, IPC mesajları)
- Performans benchmark'larını çalıştırır (`criterion`)
- CI pipeline'ında test başarısızlıklarını analiz eder ve raporlar

**Çalışma Kuralları:**
- Test fonksiyon isimleri `test_<birim>_<senaryo>_<beklenen_sonuç>` formatında olmalı
- Her Faz tamamlandığında ilgili entegrasyon testini çalıştırır
- Kırılan test, ilgili agent'a `[test-agent] FAIL:` prefix'iyle bildirilir
- Coverage hedefi: `%80` ve üzeri (kritik path'ler için `%100`)

**Bağımlı Olduğu Agentlar:** Tüm agentlar (tüketici rolünde)

**Çıktı Formatı:**
```
[test-agent] <test suite adı>
Geçen: <N> | Başarısız: <N> | Atlanan: <N>
Coverage: <%>
Kritik başarısızlık: <varsa açıklama>
```

---

## Koordinasyon Protokolü

### Faz Geçişi
1. `architect` mevcut fazın tamamlandığını onaylar
2. `progress.md` güncellenir
3. Sonraki fazın bağımlılıkları kontrol edilir
4. İlgili agentlar bilgilendirilir

### Çakışma Çözümü
- İki agent aynı modülü değiştirmek isterse → `architect` karar verir
- API sözleşmesi değişikliği → tüm bağımlı agentların onayı gerekir
- Güvenlik/unsafe kod → `architect` + `test-agent` ortak incelemesi

### İletişim Kanalları
| Durum | Yöntem |
|---|---|
| Rutin güncelleme | `progress.md` commit notu |
| Blokaj / bekleyen karar | `agents.md` altına `## Açık Sorular` bölümü |
| Kritik hata | Kod içi `// FIXME [agent-adı]:` etiketi |
| Mimari değişiklik | `architect`'e direkt bildirim + `progress.md` notu |

---

## Açık Sorular

> *(Agentlar buraya çözüme kavuşturulmamış soruları ekler)*

| # | Soru | Soran Agent | Durum |
|---|---|---|---|
| 1 | Linux'ta eBPF mi yoksa `tc` komutu mu öncelikli olmalı? | `architect` | 🔲 Açık |
| 2 | Windows'ta WFP için kernel driver gerekiyor mu, user-space yeterli mi? | `controller-agent` | 🔲 Açık |
| 3 | IPC için Unix socket mu, yoksa gRPC mi daha uygun? | `cli-agent` | 🔲 Açık |

---

## Kural Dışı Durumlar

- Hiçbir agent `progress.md`'yi `architect`'in onayı olmadan `%100 tamamlandı` olarak işaretleyemez
- `unsafe` kod `test-agent`'ın incelemesi olmadan merge edilemez
- Config şema değişiklikleri geriye dönük uyumlu olmak zorundadır (v1.0.0'a kadar)
