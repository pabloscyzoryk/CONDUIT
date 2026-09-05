//! Kod QR do logowania — do okna (SVG) i do konsoli (ASCII/ANSI).
//!
//! Telegram loguje przez QR tak: serwer wydaje token binarny, my kodujemy go
//! base64url i wsadzamy w adres `tg://login?token=...`. Użytkownik skanuje ten
//! adres OFICJALNĄ aplikacją Telegrama (Ustawienia → Urządzenia → Podłącz
//! urządzenie), a nie dowolnym czytnikiem QR.
//!
//! # Dlaczego renderujemy sami
//!
//! Biblioteka `qrcode` potrafi rysować SVG i PNG, ale to ciągnie za sobą całą
//! bibliotekę graficzną. Nam wystarcza macierz modułów — a rysowanie
//! prostokątów i bloków to kilkadziesiąt linii, za to bez zależności.
//!
//! # Pułapka, o którą łatwo się potknąć
//!
//! Czytniki QR oczekują CIEMNYCH modułów na JASNYM tle. W terminalu z ciemnym
//! motywem naiwne „ciemny moduł = znak █" daje obraz ODWRÓCONY, którego część
//! telefonów nie przeczyta. Dlatego `AsciiStyle::Ansi` maluje tło kolorami
//! (biały/czarny) i wygląda tak samo niezależnie od motywu terminala — i to
//! jest domyślny wybór dla konsoli.

use base64::Engine as _;
use qrcode::{EcLevel, QrCode};

/// Ile pustych modułów wokół kodu. Norma mówi 4 i mniej naprawdę bywa
/// nieczytelne dla telefonu trzymanego pod kątem.
pub const QUIET_ZONE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsciiStyle {
    /// Pełne bloki, ciemny moduł = `██`. Poprawne na JASNYM tle terminala.
    Blocks,
    /// Odwrotność `Blocks` — poprawne na CIEMNYM tle terminala.
    BlocksInverted,
    /// Półbloki: dwa wiersze modułów na jeden wiersz znaków. Dwa razy niższy
    /// obrazek, ta sama czytelność. Zakłada jasne tło.
    HalfBlocks,
    /// Kolory tła ANSI — wygląda poprawnie przy każdym motywie terminala.
    Ansi,
}

/// Wyrenderowany kod QR wraz z adresem, który koduje.
#[derive(Debug, Clone)]
pub struct QrRender {
    /// `tg://login?token=...`
    pub url: String,
    /// szerokość macierzy w modułach (bez marginesu)
    pub width: usize,
    /// macierz modułów, `true` = ciemny; wiersz po wierszu
    pub modules: Vec<bool>,
}

/// Buduje adres logowania z surowego tokenu.
///
/// Kodowanie to base64**url** BEZ dopełnienia — `+/=` w adresie `tg://`
/// psują parsowanie po stronie aplikacji Telegrama.
pub fn login_url(token: &[u8]) -> String {
    format!(
        "tg://login?token={}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token)
    )
}

/// Odwrotność `login_url` — przydaje się w testach i przy diagnostyce.
pub fn token_from_url(url: &str) -> Option<Vec<u8>> {
    let t = url.strip_prefix("tg://login?token=")?;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(t)
        .ok()
}

impl QrRender {
    /// Renderuje kod QR dla podanego adresu.
    ///
    /// Poziom korekcji `L` jest tu właściwy: token żyje ~30 sekund, kod jest
    /// pokazywany na ekranie (a nie drukowany i brudzony), a niższa korekcja
    /// to mniejsza macierz, czyli większe moduły i łatwiejsze skanowanie.
    pub fn new(url: &str) -> anyhow::Result<Self> {
        let code = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::L)
            .map_err(|e| anyhow::anyhow!("nie da się zbudować kodu QR: {e}"))?;
        let width = code.width();
        let modules = code
            .to_colors()
            .into_iter()
            .map(|c| c == qrcode::Color::Dark)
            .collect();
        Ok(QrRender {
            url: url.to_string(),
            width,
            modules,
        })
    }

    /// Buduje kod QR wprost z tokenu logowania.
    pub fn from_token(token: &[u8]) -> anyhow::Result<Self> {
        Self::new(&login_url(token))
    }

    #[inline]
    fn dark(&self, x: usize, y: usize) -> bool {
        if x >= self.width || y >= self.width {
            return false;
        }
        self.modules[y * self.width + x]
    }

    /// Czy moduł (z marginesem) jest ciemny? Współrzędne liczone od rogu
    /// obrazka, czyli z uwzględnieniem strefy ciszy.
    #[inline]
    fn dark_padded(&self, x: usize, y: usize) -> bool {
        if x < QUIET_ZONE || y < QUIET_ZONE {
            return false;
        }
        self.dark(x - QUIET_ZONE, y - QUIET_ZONE)
    }

    /// Bok obrazka razem ze strefą ciszy.
    #[inline]
    pub fn padded_width(&self) -> usize {
        self.width + 2 * QUIET_ZONE
    }

    /// SVG do pokazania w oknie aplikacji.
    ///
    /// Jeden `<rect>` na moduł byłby poprawny, ale przy 45×45 modułach daje
    /// dwa tysiące węzłów. Zamiast tego sklejamy poziome ciągi ciemnych modułów
    /// w jeden prostokąt — ten sam obraz, kilkakrotnie mniej węzłów.
    pub fn to_svg(&self, module_px: usize) -> String {
        let n = self.padded_width();
        let side = n * module_px;
        let mut s = String::with_capacity(4096);
        s.push_str(&format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{side}" height="{side}" viewBox="0 0 {side} {side}" shape-rendering="crispEdges" role="img" aria-label="Kod QR logowania do Telegrama">"#
        ));
        // tło MUSI być jasne — bez niego przezroczysty SVG na ciemnym motywie
        // daje kod odwrócony, a więc nieczytelny
        s.push_str(&format!(
            r##"<rect width="{side}" height="{side}" fill="#ffffff"/>"##
        ));
        s.push_str(r##"<g fill="#000000">"##);
        for y in 0..n {
            let mut x = 0;
            while x < n {
                if !self.dark_padded(x, y) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < n && self.dark_padded(x, y) {
                    x += 1;
                }
                let w = (x - start) * module_px;
                s.push_str(&format!(
                    r#"<rect x="{}" y="{}" width="{w}" height="{module_px}"/>"#,
                    start * module_px,
                    y * module_px
                ));
            }
        }
        s.push_str("</g></svg>");
        s
    }

    /// Kod QR do wypisania w konsoli.
    pub fn to_ascii(&self, style: AsciiStyle) -> String {
        let n = self.padded_width();
        let mut s = String::with_capacity(n * n + n);
        match style {
            AsciiStyle::Blocks | AsciiStyle::BlocksInverted => {
                let inv = style == AsciiStyle::BlocksInverted;
                for y in 0..n {
                    for x in 0..n {
                        let d = self.dark_padded(x, y) != inv;
                        s.push_str(if d { "██" } else { "  " });
                    }
                    s.push('\n');
                }
            }
            AsciiStyle::HalfBlocks => {
                let mut y = 0;
                while y < n {
                    for x in 0..n {
                        let top = self.dark_padded(x, y);
                        let bot = if y + 1 < n {
                            self.dark_padded(x, y + 1)
                        } else {
                            false
                        };
                        s.push(match (top, bot) {
                            (true, true) => '█',
                            (true, false) => '▀',
                            (false, true) => '▄',
                            (false, false) => ' ',
                        });
                    }
                    s.push('\n');
                    y += 2;
                }
            }
            AsciiStyle::Ansi => {
                for y in 0..n {
                    let mut cur: Option<bool> = None;
                    for x in 0..n {
                        let d = self.dark_padded(x, y);
                        if cur != Some(d) {
                            // 40 = tło czarne (moduł), 47 = tło białe (tło kodu)
                            s.push_str(if d { "\x1b[40m" } else { "\x1b[47m" });
                            cur = Some(d);
                        }
                        s.push_str("  ");
                    }
                    s.push_str("\x1b[0m\n");
                }
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adres_logowania_uzywa_base64url_bez_dopelnienia() {
        // bajty dobrane tak, żeby zwykłe base64 dało '+', '/' oraz '='
        let token = vec![0xfb, 0xff, 0xbe, 0xff];
        let url = login_url(&token);
        assert!(url.starts_with("tg://login?token="), "{url}");
        // sprawdzamy SAM token — ukośniki w „tg://" to część schematu adresu
        let enc = url.strip_prefix("tg://login?token=").unwrap();
        assert!(!enc.contains('+'), "base64url nie ma '+': {enc}");
        assert!(!enc.contains('/'), "base64url nie ma '/': {enc}");
        assert!(!enc.contains('='), "bez dopełnienia: {enc}");
        // zwykłe base64 dałoby tu „+/++/w==" — dowód, że alfabet jest właściwy
        assert_eq!(enc, "-_--_w");
        assert_eq!(token_from_url(&url).unwrap(), token);
    }

    #[test]
    fn adres_nie_z_telegrama_nie_daje_tokenu() {
        assert!(token_from_url("https://example.com").is_none());
        assert!(token_from_url("tg://login?token=###").is_none());
    }

    #[test]
    fn token_dowolnej_dlugosci_przechodzi_w_obie_strony() {
        for n in [0usize, 1, 2, 3, 16, 17, 32, 64] {
            let t: Vec<u8> = (0..n).map(|i| (i * 37 % 251) as u8).collect();
            let url = login_url(&t);
            assert_eq!(token_from_url(&url).unwrap(), t, "długość {n}");
        }
    }

    fn probny_kod() -> QrRender {
        // 32 bajty — tyle ma realny token logowania Telegrama
        let token: Vec<u8> = (0..32u8).collect();
        QrRender::from_token(&token).unwrap()
    }

    #[test]
    fn macierz_ma_spojne_wymiary() {
        let q = probny_kod();
        assert_eq!(q.modules.len(), q.width * q.width);
        // QR wersji 1 ma 21 modułów; token 32-bajtowy potrzebuje więcej
        assert!(q.width >= 21, "szerokość {}", q.width);
        assert_eq!(q.padded_width(), q.width + 8);
    }

    #[test]
    fn znacznik_pozycjonujacy_jest_na_swoim_miejscu() {
        // Bez tego kodu nie przeczyta żaden czytnik: lewy górny znacznik to
        // kwadrat 7x7 — ciemna ramka, jasna przerwa, ciemne jądro 3x3.
        let q = probny_kod();
        for i in 0..7 {
            assert!(q.dark(i, 0), "górna krawędź znacznika, x={i}");
            assert!(q.dark(0, i), "lewa krawędź znacznika, y={i}");
            assert!(q.dark(i, 6), "dolna krawędź znacznika, x={i}");
        }
        assert!(!q.dark(1, 1), "przerwa znacznika");
        assert!(q.dark(3, 3), "jądro znacznika");
    }

    #[test]
    fn strefa_ciszy_jest_pusta_ze_wszystkich_stron() {
        let q = probny_kod();
        let n = q.padded_width();
        for i in 0..n {
            for k in 0..QUIET_ZONE {
                assert!(!q.dark_padded(i, k), "górny margines");
                assert!(!q.dark_padded(i, n - 1 - k), "dolny margines");
                assert!(!q.dark_padded(k, i), "lewy margines");
                assert!(!q.dark_padded(n - 1 - k, i), "prawy margines");
            }
        }
    }

    #[test]
    fn svg_jest_kompletny_i_ma_jasne_tlo() {
        let q = probny_kod();
        let svg = q.to_svg(6);
        assert!(svg.starts_with("<svg"), "{}", &svg[..40.min(svg.len())]);
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains(r#"xmlns="http://www.w3.org/2000/svg""#));
        // jasne tło jest warunkiem czytelności na ciemnym motywie okna
        assert!(svg.contains("#ffffff"), "brak jasnego tła");
        assert!(svg.contains("#000000"), "brak ciemnych modułów");
        let side = q.padded_width() * 6;
        assert!(svg.contains(&format!(r#"width="{side}""#)));
        // sklejanie ciągów musi dać MNIEJ prostokątów niż ciemnych modułów
        let rects = svg.matches("<rect").count();
        let dark = q.modules.iter().filter(|x| **x).count();
        assert!(rects <= dark, "sklejanie nie zadziałało: {rects} > {dark}");
    }

    #[test]
    fn ascii_ma_tyle_wierszy_ile_obiecuje() {
        let q = probny_kod();
        let n = q.padded_width();

        let blocks = q.to_ascii(AsciiStyle::Blocks);
        assert_eq!(blocks.lines().count(), n);
        // każdy moduł to dwa znaki — inaczej kod byłby spłaszczony i nieczytelny
        assert!(blocks.lines().all(|l| l.chars().count() == 2 * n));

        let half = q.to_ascii(AsciiStyle::HalfBlocks);
        assert_eq!(half.lines().count(), n.div_ceil(2));
        assert!(half.lines().all(|l| l.chars().count() == n));
    }

    #[test]
    fn odwrocenie_zamienia_ciemne_z_jasnym() {
        let q = probny_kod();
        let a = q.to_ascii(AsciiStyle::Blocks);
        let b = q.to_ascii(AsciiStyle::BlocksInverted);
        assert_ne!(a, b);
        assert_eq!(
            a.matches('█').count(),
            b.lines().count() * 2 * q.padded_width() - b.matches('█').count(),
            "liczba bloków musi się dopełniać do całości obrazka"
        );
    }

    #[test]
    fn ansi_zawsze_zamyka_sekwencje_kolorow() {
        let q = probny_kod();
        let s = q.to_ascii(AsciiStyle::Ansi);
        for (i, line) in s.lines().enumerate() {
            assert!(line.ends_with("\x1b[0m"), "wiersz {i} nie resetuje koloru");
        }
        assert!(s.contains("\x1b[47m"), "brak jasnego tła");
        assert!(s.contains("\x1b[40m"), "brak ciemnych modułów");
    }

    #[test]
    fn ten_sam_token_daje_ten_sam_kod() {
        let t = vec![7u8; 32];
        let a = QrRender::from_token(&t).unwrap();
        let b = QrRender::from_token(&t).unwrap();
        assert_eq!(a.modules, b.modules);
        assert_eq!(a.url, b.url);
    }
}
