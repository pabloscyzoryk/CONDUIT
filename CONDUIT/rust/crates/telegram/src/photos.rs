//! Miniatury zdjęć profilowych czatów — pobieranie i pamięć podręczna na dysku.
//!
//! # Dlaczego pamięć podręczna jest tu obowiązkowa, a nie „miła"
//!
//! Konto ma 200+ dialogów. Każde zdjęcie to osobne wywołanie `upload.getFile`,
//! więc odświeżenie listy bez pamięci podręcznej znaczy 200 wywołań MTProto —
//! przy każdym wejściu na ekran „Kanały". Telegram odpowiada na to `FLOOD_WAIT`
//! i wycina konto na minuty. Dlatego plik pobrany raz zostaje na dysku
//! (`cache/kanaly/<chat_id>.jpg`) i nikt go już nie pobiera.
//!
//! # Jak wykrywamy podmianę zdjęcia
//!
//! Telegram nadaje każdemu zdjęciu profilowemu `photo_id`, które ZMIENIA SIĘ
//! przy podmianie obrazka. Ten identyfikator przychodzi razem z listą dialogów
//! (za darmo, bez pobierania pliku), więc porównanie z zapisanym w indeksie
//! jest pełnoprawnym znacznikiem wersji. Odświeżanie „raz na dobę" nie jest
//! potrzebne — poza jednym przypadkiem: gdy pobranie się NIE UDAŁO. Wtedy
//! zapisujemy porażkę i ponawiamy próbę dopiero po dobie, żeby zerwana sieć
//! nie zamieniła listy kanałów w karuzelę nieudanych wywołań.
//!
//! # Rozmiar
//!
//! Świadomie `big: false` — Telegram oddaje wtedy wariant ~160×160 px, kilka
//! kilobajtów. Kółko na liście ma 38 px; pobieranie wersji 640×640 dla każdego
//! z 200 czatów byłoby marnowaniem i czasu, i limitów.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use grammers_client::session::types::PeerRef;
use grammers_client::{tl, Client};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tracing::{debug, warn};

/// Podkatalog pamięci podręcznej wewnątrz katalogu roboczego.
pub const KATALOG: &str = "cache/kanaly";

/// Ile pobrań naraz. Przeglądarka i tak zamawia obrazki porcjami, ale to jest
/// twarda granica po NASZEJ stronie: gdyby ktoś odpytał wszystkie 200 naraz
/// (skrypt, odświeżenie z wyłączonym cache), Telegram nie dostanie 200
/// równoległych `upload.getFile`.
const ROWNOLEGLE: usize = 4;

/// Po jakim czasie ponawiamy próbę po NIEUDANYM pobraniu.
const PONOW_PO_MS: i64 = 24 * 60 * 60 * 1000;

/// Górna granica rozmiaru miniatury. Zabezpieczenie przed pobraniem czegoś,
/// czego się nie spodziewamy (zdjęcie profilowe „małe" ma kilka kB).
const MAX_BAJTOW: usize = 2 * 1024 * 1024;

// ============================================================
//  INDEKS
// ============================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Wpis {
    /// `photo_id` z Telegrama w chwili pobrania; `None` = czat nie miał zdjęcia
    #[serde(default)]
    photo_id: Option<i64>,
    /// czy na dysku leży użyteczny plik
    #[serde(default)]
    ok: bool,
    /// kiedy ostatnio próbowaliśmy (ms epoki)
    #[serde(default)]
    ts: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Indeks {
    /// klucz to `chat_id` jako tekst — JSON nie ma liczbowych kluczy
    #[serde(default)]
    wpisy: HashMap<String, Wpis>,
}

fn teraz_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ============================================================
//  PAMIĘĆ PODRĘCZNA
// ============================================================

pub struct PhotoCache {
    dir: PathBuf,
    indeks: Mutex<Indeks>,
    limit: Arc<Semaphore>,
}

impl std::fmt::Debug for PhotoCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoCache")
            .field("katalog", &self.dir)
            .field("wpisow", &self.indeks.lock().wpisy.len())
            .finish()
    }
}

impl PhotoCache {
    /// Otwiera (i w razie potrzeby zakłada) pamięć podręczną w katalogu roboczym.
    pub fn new(root: &Path) -> Self {
        let dir = root.join(KATALOG);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!(katalog = %dir.display(), %e, "Telegram: nie udało się założyć katalogu miniatur");
        }
        let indeks = std::fs::read(dir.join("index.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Indeks>(&b).ok())
            .unwrap_or_default();
        PhotoCache {
            dir,
            indeks: Mutex::new(indeks),
            limit: Arc::new(Semaphore::new(ROWNOLEGLE)),
        }
    }

    /// Ścieżka pliku miniatury (może nie istnieć).
    pub fn path(&self, chat_id: i64) -> PathBuf {
        self.dir.join(format!("{chat_id}.jpg"))
    }

    fn zapisz_indeks(&self) {
        let dane = { serde_json::to_vec_pretty(&*self.indeks.lock()) };
        let Ok(dane) = dane else { return };
        let tmp = self.dir.join("index.json.tmp");
        if std::fs::write(&tmp, &dane).is_ok() {
            let _ = std::fs::rename(&tmp, self.dir.join("index.json"));
        }
    }

    /// Czy plik na dysku odpowiada BIEŻĄCEJ wersji zdjęcia?
    ///
    /// Publiczne, bo to jest cała odpowiedź na pytanie „czy trzeba iść do
    /// Telegrama": uchwyt HTTP sprawdza to sam i przy trafieniu w pamięć
    /// podręczną nie zawraca głowy zadaniu w tle.
    pub fn aktualny(&self, chat_id: i64, photo_id: Option<i64>) -> bool {
        let i = self.indeks.lock();
        let Some(w) = i.wpisy.get(&chat_id.to_string()) else {
            return false;
        };
        w.ok && w.photo_id == photo_id && self.path(chat_id).is_file()
    }

    /// Czy wolno ponowić próbę po wcześniejszej porażce?
    fn wolno_ponowic(&self, chat_id: i64, photo_id: Option<i64>) -> bool {
        let i = self.indeks.lock();
        match i.wpisy.get(&chat_id.to_string()) {
            None => true,
            // zdjęcie się zmieniło — próbujemy od razu, niezależnie od porażki
            Some(w) if w.photo_id != photo_id => true,
            Some(w) if w.ok => true,
            Some(w) => teraz_ms() - w.ts >= PONOW_PO_MS,
        }
    }

    fn odnotuj(&self, chat_id: i64, photo_id: Option<i64>, ok: bool) {
        self.indeks.lock().wpisy.insert(
            chat_id.to_string(),
            Wpis {
                photo_id,
                ok,
                ts: teraz_ms(),
            },
        );
        self.zapisz_indeks();
    }

    /// Zwraca ścieżkę do miniatury, pobierając ją tylko wtedy, gdy trzeba.
    ///
    /// `Ok(None)` znaczy „ten czat nie ma zdjęcia" — i to jest normalny wynik,
    /// nie błąd. Wołający ma wtedy oddać 404, a interfejs zostawić literkę.
    pub async fn ensure(
        &self,
        client: &Client,
        chat_id: i64,
        peer: PeerRef,
        photo_id: Option<i64>,
    ) -> anyhow::Result<Option<PathBuf>> {
        let Some(pid) = photo_id else {
            // Czat bez zdjęcia. Zapamiętujemy to, żeby nie wracać z pytaniem
            // do Telegrama przy każdym odświeżeniu listy.
            if !self.aktualny(chat_id, None) {
                let _ = std::fs::remove_file(self.path(chat_id));
                self.odnotuj(chat_id, None, false);
            }
            return Ok(None);
        };

        let sciezka = self.path(chat_id);
        if self.aktualny(chat_id, photo_id) {
            debug!(czat = chat_id, "Telegram: miniatura z pamięci podręcznej");
            return Ok(Some(sciezka));
        }
        if !self.wolno_ponowic(chat_id, photo_id) {
            return Ok(None);
        }

        let _p = self.limit.clone().acquire_owned().await?;
        // Druga kontrola po przejściu przez semafor: gdy kilka żądań na ten sam
        // czat czekało w kolejce, pierwsze już pobrało plik.
        if self.aktualny(chat_id, photo_id) {
            return Ok(Some(sciezka));
        }

        match self.pobierz(client, peer, pid).await {
            Ok(bajty) if !bajty.is_empty() => {
                let tmp = self.dir.join(format!("{chat_id}.jpg.tmp"));
                std::fs::write(&tmp, &bajty)?;
                std::fs::rename(&tmp, &sciezka)?;
                self.odnotuj(chat_id, photo_id, true);
                debug!(
                    czat = chat_id,
                    bajtow = bajty.len(),
                    "Telegram: miniatura pobrana"
                );
                Ok(Some(sciezka))
            }
            Ok(_) => {
                self.odnotuj(chat_id, photo_id, false);
                Ok(None)
            }
            Err(e) => {
                warn!(czat = chat_id, %e, "Telegram: nie udało się pobrać miniatury");
                self.odnotuj(chat_id, photo_id, false);
                Err(e)
            }
        }
    }

    /// Samo pobranie — MAŁY wariant zdjęcia (`big: false`).
    async fn pobierz(
        &self,
        client: &Client,
        peer: PeerRef,
        photo_id: i64,
    ) -> anyhow::Result<Vec<u8>> {
        let zdjecie = grammers_client::media::ChatPhoto {
            raw: tl::enums::InputFileLocation::InputPeerPhotoFileLocation(
                tl::types::InputPeerPhotoFileLocation {
                    big: false,
                    peer: peer.into(),
                    photo_id,
                },
            ),
        };
        let mut it = client.iter_download(&zdjecie);
        let mut buf = Vec::new();
        while let Some(kawalek) = it.next().await? {
            buf.extend_from_slice(&kawalek);
            anyhow::ensure!(
                buf.len() <= MAX_BAJTOW,
                "miniatura większa niż {MAX_BAJTOW} B"
            );
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tymczasowy(nazwa: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("conduit_foto_{nazwa}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn brak_zdjecia_zapamietuje_sie_i_nie_wraca_do_telegrama() {
        let root = tymczasowy("brak");
        let c = PhotoCache::new(&root);
        // czat bez zdjęcia: nic na dysku, ale wpis w indeksie jest
        c.odnotuj(-100, None, false);
        assert!(!c.aktualny(-100, None), "bez pliku nie ma czego serwować");
        // ...i ponowienie nie jest dozwolone przed upływem doby
        assert!(!c.wolno_ponowic(-100, None));
        // za to ZMIANA zdjęcia natychmiast otwiera drogę do pobrania
        assert!(c.wolno_ponowic(-100, Some(7)));
    }

    #[test]
    fn zmiana_photo_id_uniewaznia_plik() {
        let root = tymczasowy("wersja");
        let c = PhotoCache::new(&root);
        std::fs::write(c.path(-200), b"udawany-jpeg").unwrap();
        c.odnotuj(-200, Some(1), true);
        assert!(
            c.aktualny(-200, Some(1)),
            "ta sama wersja = plik jest dobry"
        );
        assert!(
            !c.aktualny(-200, Some(2)),
            "nowe photo_id musi wymusić pobranie"
        );
    }

    #[test]
    fn indeks_przezywa_ponowne_otwarcie() {
        let root = tymczasowy("indeks");
        {
            let c = PhotoCache::new(&root);
            std::fs::write(c.path(-300), b"x").unwrap();
            c.odnotuj(-300, Some(42), true);
        }
        let c = PhotoCache::new(&root);
        assert!(
            c.aktualny(-300, Some(42)),
            "restart programu nie może kasować pamięci podręcznej"
        );
    }
}
