
use crate::aktuator::{self, ILE};
use std::collections::BTreeMap;

/// Puls z ticku (`Engine::on_tick`).
pub const PULS_TICK: u8 = 0;
/// Puls z wiadomości kanału (`Engine::on_message`).
pub const PULS_WIADOMOSC: u8 = 1;

const SUFIT_PRZYKLADOW: usize = 40;

/// Jeden zamiar w pulsie. Klucz źródła to PARA `(zrodlo, linia)`:
/// dla lejków (`try_modify`, `cancel_pendings`, …) rodzinę odróżnia dopiero
/// linia wołającego.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Wpis {
    pub akt: u8,
    pub id: u64,
    pub zrodlo: u16,
    pub linia: u32,
}

#[derive(Default)]
pub struct ArbiterCieniowy {
    // --- stan bieżącego pulsu ---
    puls_otwarty: bool,
    puls_rodzaj: u8,
    puls_ts: i64,
    biezace: Vec<Wpis>,

    // --- agregaty przebiegu ---
    pub pulsy: u64,
    pub pulsy_tick: u64,
    pub pulsy_wiadomosc: u64,
    pub zamiary: u64,
    pub pulsy_z_zamiarem: u64,
    /// LICZBA ROZSTRZYGAJĄCA: pulsy z ≥2 zamiarami na jednym aktuatorze.
    pub pulsy_z_konfliktem: u64,
    /// Jak wyżej, ale wymagane ≥2 RÓŻNE źródła.
    pub pulsy_z_konfliktem_roznych: u64,
    pub instancje_konfliktu: u64,
    pub instancje_konfliktu_roznych: u64,
    pub max_zamiarow_na_aktuator: u32,

    zam_akt: [u64; ILE],
    konf_akt: [u64; ILE],
    konf_roznych_akt: [u64; ILE],
    pulsy_konf_akt: [u64; ILE],

    /// `(aktuator, źródło A, źródło B) → ile razy` — A < B, więc para jest
    /// nieuporządkowana i nie zależy od kolejności zgłoszeń.
    pary: BTreeMap<(u8, (u16, u32), (u16, u32)), u64>,
    zrodla: BTreeMap<(u16, u32), (u64, [u64; ILE])>,
    przyklady: Vec<String>,
}

impl ArbiterCieniowy {
    pub fn nowy() -> Self {
        Self::default()
    }

    /// Zamyka poprzedni puls i otwiera nowy.
    pub fn puls(&mut self, ts: i64, rodzaj: u8) {
        self.zamknij_puls();
        self.puls_otwarty = true;
        self.puls_rodzaj = rodzaj;
        self.puls_ts = ts;
        self.biezace.clear();
    }

    /// Zgłasza zamiar w bieżącym pulsie.
    ///
    /// Zamiar bez otwartego pulsu (np. z odtworzenia stanu przed pierwszym
    /// tickiem) trafia do pulsu syntetycznego — nie ginie i nie zawyża
    /// konfliktów, bo taki puls zamknie się przy najbliższym `puls()`.
    pub fn zamiar(&mut self, akt: u8, id: u64, zrodlo: u16, linia: u32) {
        if !self.puls_otwarty {
            self.biezace.clear();
            self.puls_otwarty = true;
            self.puls_rodzaj = PULS_TICK;
        }
        self.biezace.push(Wpis {
            akt,
            id,
            zrodlo,
            linia,
        });
        self.zamiary += 1;
        let i = akt as usize;
        if i < ILE {
            self.zam_akt[i] += 1;
        }
        let e = self
            .zrodla
            .entry((zrodlo, linia))
            .or_insert((0, [0u64; ILE]));
        e.0 += 1;
        if i < ILE {
            e.1[i] += 1;
        }
    }

    fn zamknij_puls(&mut self) {
        if !self.puls_otwarty {
            return;
        }
        self.puls_otwarty = false;
        self.pulsy += 1;
        if self.puls_rodzaj == PULS_WIADOMOSC {
            self.pulsy_wiadomosc += 1;
        } else {
            self.pulsy_tick += 1;
        }
        if self.biezace.is_empty() {
            return;
        }
        self.pulsy_z_zamiarem += 1;

        // Klucz z samych liczb całkowitych — kolejność nie zależy od
        // zaokrąglenia ani od kolejności iteracji po mapie (N19 z §7).
        self.biezace.sort_unstable();

        let mut byl_konflikt = false;
        let mut byl_konflikt_roznych = false;
        let mut widziane_akt = [false; ILE];

        let n = self.biezace.len();
        let mut i = 0usize;
        while i < n {
            let a = self.biezace[i];
            let mut j = i + 1;
            while j < n && self.biezace[j].akt == a.akt && self.biezace[j].id == a.id {
                j += 1;
            }
            let ile = (j - i) as u32;
            if ile > self.max_zamiarow_na_aktuator {
                self.max_zamiarow_na_aktuator = ile;
            }
            if ile >= 2 {
                byl_konflikt = true;
                self.instancje_konfliktu += 1;
                let ia = a.akt as usize;
                if ia < ILE {
                    self.konf_akt[ia] += 1;
                    if !widziane_akt[ia] {
                        widziane_akt[ia] = true;
                        self.pulsy_konf_akt[ia] += 1;
                    }
                }
                // czy w grupie są DWA RÓŻNE źródła?
                let mut rozne = false;
                for k in (i + 1)..j {
                    if (self.biezace[k].zrodlo, self.biezace[k].linia)
                        != (self.biezace[i].zrodlo, self.biezace[i].linia)
                    {
                        rozne = true;
                        break;
                    }
                }
                if rozne {
                    byl_konflikt_roznych = true;
                    self.instancje_konfliktu_roznych += 1;
                    if ia < ILE {
                        self.konf_roznych_akt[ia] += 1;
                    }
                    // wszystkie pary różnych źródeł w grupie
                    for p in i..j {
                        for q in (p + 1)..j {
                            let sp = (self.biezace[p].zrodlo, self.biezace[p].linia);
                            let sq = (self.biezace[q].zrodlo, self.biezace[q].linia);
                            if sp == sq {
                                continue;
                            }
                            let (x, y) = if sp <= sq { (sp, sq) } else { (sq, sp) };
                            *self.pary.entry((a.akt, x, y)).or_insert(0) += 1;
                        }
                    }
                    if self.przyklady.len() < SUFIT_PRZYKLADOW {
                        let mut s = String::new();
                        s.push_str(&format!(
                            "ts={} akt={} id={} zamiary={} :",
                            self.puls_ts,
                            aktuator::nazwa(a.akt),
                            a.id,
                            ile
                        ));
                        for k in i..j {
                            s.push_str(&format!(
                                " [{}@{}]",
                                crate::zrodlo::nazwa(self.biezace[k].zrodlo),
                                self.biezace[k].linia
                            ));
                        }
                        self.przyklady.push(s);
                    }
                }
            }
            i = j;
        }
        if byl_konflikt {
            self.pulsy_z_konfliktem += 1;
        }
        if byl_konflikt_roznych {
            self.pulsy_z_konfliktem_roznych += 1;
        }
    }

    /// Domyka ostatni puls i składa raport tekstowy.
    pub fn raport(&mut self, naglowek: &str) -> String {
        self.zamknij_puls();
        let mut s = String::with_capacity(8192);
        s.push_str("# ARBITER CIENIOWY — ETAP E0 (pomiar bez zmiany zachowania)\n");
        s.push_str(naglowek);
        s.push('\n');
        s.push_str("\n[PULSY]\n");
        s.push_str(&format!("pulsy_ogolem={}\n", self.pulsy));
        s.push_str(&format!("pulsy_tick={}\n", self.pulsy_tick));
        s.push_str(&format!("pulsy_wiadomosc={}\n", self.pulsy_wiadomosc));
        s.push_str(&format!("zamiary_ogolem={}\n", self.zamiary));
        s.push_str(&format!("pulsy_z_zamiarem={}\n", self.pulsy_z_zamiarem));
        s.push_str("\n[LICZBA ROZSTRZYGAJACA]\n");
        s.push_str(&format!(
            "pulsy_z_konfliktem={}   # >=2 zamiary na TYM SAMYM aktuatorze\n",
            self.pulsy_z_konfliktem
        ));
        s.push_str(&format!(
            "pulsy_z_konfliktem_roznych_zrodel={}   # >=2 RÓŻNE źródła\n",
            self.pulsy_z_konfliktem_roznych
        ));
        s.push_str(&format!(
            "instancje_konfliktu={}\n",
            self.instancje_konfliktu
        ));
        s.push_str(&format!(
            "instancje_konfliktu_roznych_zrodel={}\n",
            self.instancje_konfliktu_roznych
        ));
        s.push_str(&format!(
            "max_zamiarow_na_jeden_aktuator={}\n",
            self.max_zamiarow_na_aktuator
        ));

        s.push_str("\n[AKTUATORY]  aktuator;zamiary;instancje_konfliktu;instancje_roznych;pulsy_z_konfliktem\n");
        for i in 0..ILE {
            s.push_str(&format!(
                "{};{};{};{};{}\n",
                aktuator::NAZWY[i],
                self.zam_akt[i],
                self.konf_akt[i],
                self.konf_roznych_akt[i],
                self.pulsy_konf_akt[i]
            ));
        }

        s.push_str("\n[PISARZE ZMIERZENI W RUCHU]  zrodlo;linia;zamiary;rozbicie_po_aktuatorach\n");
        let mut lista: Vec<((u16, u32), (u64, [u64; ILE]))> =
            self.zrodla.iter().map(|(k, v)| (*k, *v)).collect();
        lista.sort_unstable_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(&b.0)));
        for ((z, l), (n, akt)) in lista {
            let mut roz = String::new();
            for i in 0..ILE {
                if akt[i] > 0 {
                    roz.push_str(&format!("{}={} ", aktuator::NAZWY[i], akt[i]));
                }
            }
            s.push_str(&format!(
                "{};{};{};{}\n",
                crate::zrodlo::nazwa(z),
                l,
                n,
                roz.trim_end()
            ));
        }

        s.push_str("\n[PARY ZRODEL W KONFLIKCIE]  aktuator;A;B;ile\n");
        let mut pary: Vec<((u8, (u16, u32), (u16, u32)), u64)> =
            self.pary.iter().map(|(k, v)| (*k, *v)).collect();
        pary.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for ((a, x, y), n) in pary {
            s.push_str(&format!(
                "{};{}@{};{}@{};{}\n",
                aktuator::nazwa(a),
                crate::zrodlo::nazwa(x.0),
                x.1,
                crate::zrodlo::nazwa(y.0),
                y.1,
                n
            ));
        }

        s.push_str("\n[PRZYKLADY]\n");
        for p in &self.przyklady {
            s.push_str(p);
            s.push('\n');
        }
        s
    }
}

#[cfg(test)]
mod testy {
    use super::*;
    use crate::aktuator::{A_CEL, A_STOP};
    use crate::zrodlo::L_TRY_MODIFY;

    #[test]
    fn dwa_zrodla_na_tym_samym_stopie_to_konflikt() {
        let mut a = ArbiterCieniowy::nowy();
        a.puls(1, PULS_TICK);
        a.zamiar(A_STOP, 7, L_TRY_MODIFY, 8510);
        a.zamiar(A_STOP, 7, L_TRY_MODIFY, 8536);
        let r = a.raport("test");
        assert_eq!(a.pulsy_z_konfliktem, 1);
        assert_eq!(a.pulsy_z_konfliktem_roznych, 1);
        assert!(r.contains("pulsy_z_konfliktem=1"));
    }

    #[test]
    fn ten_sam_pisarz_dwa_razy_to_nie_spor() {
        let mut a = ArbiterCieniowy::nowy();
        a.puls(1, PULS_TICK);
        a.zamiar(A_STOP, 7, L_TRY_MODIFY, 8510);
        a.zamiar(A_STOP, 7, L_TRY_MODIFY, 8510);
        a.raport("test");
        assert_eq!(a.pulsy_z_konfliktem, 1);
        assert_eq!(a.pulsy_z_konfliktem_roznych, 0);
    }

    #[test]
    fn rozne_aktuatory_i_rozne_bilety_nie_koliduja() {
        let mut a = ArbiterCieniowy::nowy();
        a.puls(1, PULS_TICK);
        a.zamiar(A_STOP, 7, L_TRY_MODIFY, 8510);
        a.zamiar(A_CEL, 7, L_TRY_MODIFY, 6479);
        a.zamiar(A_STOP, 8, L_TRY_MODIFY, 8536);
        a.raport("test");
        assert_eq!(a.pulsy_z_konfliktem, 0);
        assert_eq!(a.zamiary, 3);
    }

    #[test]
    fn konflikt_liczy_sie_raz_na_puls_ale_instancje_osobno() {
        let mut a = ArbiterCieniowy::nowy();
        a.puls(1, PULS_TICK);
        a.zamiar(A_STOP, 7, L_TRY_MODIFY, 8510);
        a.zamiar(A_STOP, 7, L_TRY_MODIFY, 8536);
        a.zamiar(A_STOP, 9, L_TRY_MODIFY, 8510);
        a.zamiar(A_STOP, 9, L_TRY_MODIFY, 8613);
        a.puls(2, PULS_TICK);
        a.raport("test");
        assert_eq!(a.pulsy_z_konfliktem, 1);
        assert_eq!(a.instancje_konfliktu, 2);
        assert_eq!(a.pulsy, 2);
    }
}
