//! Wykonanie komend z interfejsu.
//!
//! Podział odpowiedzialności jest ostry:
//!  * komendy KONFIGURACYJNE serwer wykonuje sam — zmienia stan, zapisuje plik
//!    i rozsyła deltę, więc zmiana zrobiona w oknie natywnym pojawia się
//!    natychmiast w przeglądarce i odwrotnie,
//!  * komendy HANDLOWE serwer przekazuje do środowiska uruchomieniowego
//!    (`Runtime`, czyli most do MT5). Bez podłączonego brokera kończą się
//!    jawnym błędem w `ack` — nigdy cichym powodzeniem.

use crate::coalesce::{Section, Sections};
use crate::proto::Command;
use crate::state::StateHandle;
use crate::store::{ChannelsDoc, SettingsDoc, SmtpDoc};
use crate::{settings_map, ui};
use anyhow::Result;
use serde_json::Value;

pub fn apply(st: &StateHandle, cmd: &Command) -> Result<()> {
    apply_scoped(st, cmd, None)
}

/// Validate before any side effect, including inserting a simulated message.
/// Equality is against the original render token; A->B->A gets a NEW token.
pub fn validate_account_session(st: &StateHandle, cmd: &Command, account_session: Option<&str>) -> Result<()> {
    if !cmd.needs_runtime() { return Ok(()); }
    let permitted = st.read(|s| {
        if s.settings.get("mt5_follow_terminal_account").and_then(Value::as_bool) != Some(true) {
            return true;
        }
        s.connection.mt5 == "connected"
            && s.connection.account_verified == "ok"
            && !s.connection.account_session.is_empty()
            && account_session == Some(s.connection.account_session.as_str())
    });
    if !permitted {
        anyhow::bail!("Konto lub sesja MT5 zmieniły się od utworzenia polecenia — odśwież widok i utwórz nowe polecenie (accountSession)");
    }
    Ok(())
}

pub fn apply_scoped(st: &StateHandle, cmd: &Command, account_session: Option<&str>) -> Result<()> {
    validate_account_session(st, cmd, account_session)?;
    match cmd {
        // ---------------- konfiguracja ----------------
        Command::SetMode { mode } => {
            let rt = st.runtime.read().clone();
            if rt.defer_mode_change(*mode, st, account_session)? {
                return Ok(()); // Accepted for verification, never an applied-mode acknowledgement.
            }
            commit_mode_change(st, *mode)
        }

        Command::ApplyPreset { id, values } => {
            // preset z dysku ma pierwszeństwo — to ten, który napisał użytkownik
            let z_dysku = st
                .workspace
                .load_presets()
                .into_iter()
                .find(|p| p.name.eq_ignore_ascii_case(id));

            let (as_json, nazwa, opis) = match z_dysku {
                Some(p) => (
                    serde_json::to_value(&p.settings)?,
                    p.name.clone(),
                    p.description.clone(),
                ),
                None => match values {
                    Some(v) => (
                        v.clone(),
                        id.clone(),
                        "preset z katalogu interfejsu".to_string(),
                    ),
                    None => anyhow::bail!("nie ma presetu „{id}”"),
                },
            };

            // Preset jest zapisany w kluczach SILNIKA, a `core_from_ui` czyta
            // klucze PANELU. Kilkanaście pól (m.in. `tp_schedule`,
            // `pending_lifetime`, `trail_start`) ma w panelu inne nazwy i bez
            // tego tłumaczenia wracało do wartości domyślnych — bot handlował
            // inną konfiguracją, niż nazwa presetu obiecywała.
            let mut dla_panelu = settings_map::preset_to_ui(&as_json);
            let lot = settings_map::preset_lot(&as_json);
            st.update(Sections::one(Section::Settings), |s| {
                preserve_follow_terminal_account(&s.settings, &mut dla_panelu);
                settings_map::merge_patch(&mut s.settings, &dla_panelu);
                if let Some(l) = lot.clone() {
                    s.lot = l;
                }
                s.preset_id = nazwa.clone();
            });
            persist_settings(st)?;
            st.log(
                "commands",
                "success",
                format!("Wczytano preset {nazwa}"),
                opis,
            );
            zglos_niespojnosci(st, &format!("preset {nazwa}"));
            Ok(())
        }

        Command::ResetSettings => {
            let mut domyslne = serde_json::to_value(conduit_core::Settings::default())?;
            st.update(Sections::one(Section::Settings), |s| {
                preserve_follow_terminal_account(&s.settings, &mut domyslne);
                // Wygląd panelu NIE jest ustawieniem silnika i nie ma powodu,
                // żeby „przywróć domyślne" gasiło użytkownikowi motyw.
                let wyglad: Vec<(String, serde_json::Value)> = ["ui_theme", "ui_palette"]
                    .iter()
                    .filter_map(|k| s.settings.get(*k).map(|v| (k.to_string(), v.clone())))
                    .collect();
                s.settings = domyslne.clone();
                if let Some(o) = s.settings.as_object_mut() {
                    for (k, v) in wyglad {
                        o.insert(k, v);
                    }
                }
                s.preset_id.clear();
            });
            persist_settings(st)?;
            st.log(
                "commands",
                "info",
                "Ustawienia przywrócone",
                "wartości domyślne silnika",
            );
            Ok(())
        }

        Command::SetLot { lot } => {
            st.update(Sections::one(Section::Settings), |s| {
                s.lot = lot.clone();
                // Odbicie w dokumencie ustawień, żeby obie strony mówiły to
                // samo. Bez tego zmiana lota w panelu wracałaby do poprzedniej
                // wartości przy najbliższym zapisie ustawień (karta i dokument
                // były dwoma źródłami prawdy, które nikt nie synchronizował).
                settings_map::merge_patch(
                    &mut s.settings,
                    &serde_json::json!({
                        "lot_mode_percent": lot.mode == "percent",
                        "lot_fixed": lot.fixed,
                        "lot_percent": lot.percent,
                    }),
                );
            });
            persist_settings(st)?;
            Ok(())
        }

        Command::SetBinding { channel_id, patch } => {
            // Bindings I Settings: zaznaczenie „wysyłaj tu podsumowania" przy
            // kafelku zmienia zarazem listę odbiorców na karcie powiadomień.
            st.update(Sections::two(Section::Bindings, Section::Settings), |s| {
                let key = channel_id.to_string();
                let mut cur = s.bindings.remove(&key).unwrap_or(ui::ChannelBinding {
                    channel_id: *channel_id,
                    monitored: false,
                    notify: false,
                    format: String::new(),
                    topics: Default::default(),
                });
                if let Some(o) = patch.as_object() {
                    if let Some(v) = o.get("monitored").and_then(|v| v.as_bool()) {
                        cur.monitored = v;
                    }
                    if let Some(v) = o.get("notify").and_then(|v| v.as_bool()) {
                        cur.notify = v;
                    }
                    // DWA KSZTAŁTY NA DRUCIE, JEDEN W STANIE.
                    //
                    // Panel nowszy niż serwer wysyła `format` (napis) i dla
                    // zgodności DOKŁADA `formats` (listę). Panel starszy —
                    // tylko `formats`. Czytamy oba, `format` wygrywa; bez tego
                    // przypisanie formatu przepadałoby po cichu przy jednej
                    // z kombinacji wersji.
                    if let Some(v) = o.get("format").or_else(|| o.get("formats")) {
                        cur.format = ui::pierwszy_format(v, "kanał");
                    }
                    if let Some(serde_json::Value::Object(t)) = o.get("topics") {
                        cur.topics = t
                            .iter()
                            .map(|(k, v)| {
                                (k.clone(), ui::pierwszy_format(v, &format!("temat {k}")))
                            })
                            .filter(|(_, f)| !f.is_empty())
                            .collect();
                    }
                }
                s.bindings.insert(key, cur);
                zsynchronizuj_odbiorcow_z_powiazan(s);
            });
            let doc = ChannelsDoc {
                bindings: st.read(|s| s.bindings.clone()),
            };
            st.workspace.save_channels(&doc)?;
            persist_settings(st)?;
            Ok(())
        }

        // ŁAŃCUCHY — pełny zapis zbioru.
        //
        // Odrzucamy zbiór, w którym `aktywny` wskazuje nieistniejący łańcuch:
        // bot bez aktywnego łańcucha nie ma jak zdecydować, czym handlować,
        // i po cichu przestałby brać sygnały. Lepiej odmówić zapisu i to
        // powiedzieć, niż przyjąć i zamilknąć.
        Command::SetLancuchy {
            lancuchy,
            aktywny_ea,
        } => {
            if lancuchy.aktywny().is_none() {
                anyhow::bail!(
                    "łańcuch „{}” nie istnieje na liście — nie zapisuję zbioru,                      w którym nic nie jest aktywne",
                    lancuchy.aktywny
                );
            }
            let tryb_teraz = st.read(|s| s.mode);
            if aktywny_ea.is_some() && !tryb_teraz.wlasny_lancuch() {
                let powod = format!(
                    "ODMOWA zapisu łańcuchów ze wskaźnikiem AUTO-EA: bot pracuje w trybie {}, \
                     a pole `aktywnyEa` należy do trybu AUTO-EA. Nic nie zostało zapisane — \
                     przełącz tryb albo wyślij zapis bez tego pola.",
                    mode_label(tryb_teraz)
                );
                st.log(
                    "settings",
                    "error",
                    "Odmowa: wskaźnik AUTO-EA spoza trybu EA",
                    powod.clone(),
                );
                anyhow::bail!("{powod}");
            }
            let ea = match aktywny_ea {
                None => st.read(|s| s.aktywny_ea.clone()),
                Some(n) if n.is_empty() => String::new(),
                Some(n) => {
                    if !lancuchy.lista.iter().any(|l| &l.nazwa == n) {
                        anyhow::bail!(
                            "łańcuch AUTO-EA „{n}” nie istnieje na liście — nie zapisuję                              wskazania, które pokazuje w pustkę"
                        );
                    }
                    n.clone()
                }
            };
            st.update(Sections::one(Section::Settings), |s| {
                s.lancuchy = lancuchy.clone();
                s.aktywny_ea = ea.clone();
            });
            st.workspace.save_lancuchy(lancuchy, &ea)?;
            st.log(
                "settings",
                "success",
                if ea.is_empty() {
                    format!("Zapisano łańcuchy (aktywny: {})", lancuchy.aktywny)
                } else {
                    format!(
                        "Zapisano łańcuchy (aktywny: {}, AUTO-EA: {ea})",
                        lancuchy.aktywny
                    )
                },
                opis_lancucha(lancuchy),
            );
            zglos_niespojnosci(st, &format!("zapis łańcucha {}", lancuchy.aktywny));
            Ok(())
        }

        // PRZEŁĄCZENIE AKTYWNEGO ŁAŃCUCHA — POLE WYBIERA TRYB (projekt EA-2).
        //
        // W AUTO-EA piszemy do `aktywny_ea`, w pozostałych trybach do
        // `lancuchy.aktywny`. IZOLACJA JEST TU SEDNEM: zmiana zrobiona
        // w AUTO-EA nie ma prawa ruszyć składu, którym gra AUTO, i odwrotnie
        // — inaczej przełączenie trybu tam i z powrotem cicho przepisywałoby
        // konfigurację drugiej strony.
        Command::SetAktywnyLancuch { nazwa } => {
            let (ok, doc, tryb) = st.read(|s| {
                let ok = s.lancuchy.lista.iter().any(|l| &l.nazwa == nazwa);
                (ok, s.lancuchy.clone(), s.mode)
            });
            if !ok {
                anyhow::bail!("nie ma łańcucha „{nazwa}”");
            }
            let ea_tryb = tryb.wlasny_lancuch();
            let mut nowe = doc;
            let mut ea = st.read(|s| s.aktywny_ea.clone());
            if ea_tryb {
                ea = nazwa.clone();
            } else {
                nowe.aktywny = nazwa.clone();
            }
            st.update(Sections::one(Section::Settings), |s| {
                s.lancuchy = nowe.clone();
                s.aktywny_ea = ea.clone();
            });
            st.workspace.save_lancuchy(&nowe, &ea)?;
            st.log(
                "settings",
                "success",
                if ea_tryb {
                    format!("Aktywny łańcuch EA: {nazwa}")
                } else {
                    format!("Aktywny łańcuch: {nazwa}")
                },
                format!(
                    "{}

Zmiana wchodzi w życie przy najbliższym podłączeniu do MT5 —                      silniki są tworzone razem z mostem, więc przebudowa w locie                      zostawiłaby koszyki bez opiekuna.",
                    opis_lancucha_nazwa(&nowe, nazwa)
                ),
            );
            zglos_niespojnosci(st, &format!("aktywny łańcuch {nazwa}"));
            Ok(())
        }

        Command::SetEmail { email } => {
            // Hasło NIE mieszka w stanie ani w `smtp.json` — jego miejscem jest
            // `secrets.json`. Puste pole z UI znaczy „zostaw stare hasło",
            // bo interfejs nigdy go nie dostaje i nie miałby czego odesłać.
            if !email.pass.is_empty() {
                let mut sek = st.workspace.load_secrets();
                sek.smtp.password = email.pass.clone().into();
                st.workspace.save_secrets(&sek)?;
                st.log(
                    "email",
                    "info",
                    "Zapisano hasło SMTP",
                    "trafiło do secrets.json (osobny plik, prawa tylko dla właściciela)",
                );
            }
            st.update(Sections::one(Section::Settings), |s| {
                s.email = email.clone();
                s.email.pass.clear();
            });
            persist_smtp(st)?;
            odswiez_powiadamiacz(st);
            Ok(())
        }

        Command::SendTestEmail => {
            let n = st
                .notifier
                .read()
                .clone()
                .ok_or_else(|| anyhow::anyhow!("powiadomienia e-mail nie są uruchomione"))?;
            // konfiguracja mogła się zmienić sekundę wcześniej
            n.reconfigure(crate::notify::setup_from_state(st));
            let opis = n.send_test(st)?;
            st.emit(ui::UiEvent::Toast {
                level: "success".into(),
                title: "Mail testowy wysłany".into(),
                text: opis,
            });
            Ok(())
        }

        Command::SendTestNotify => {
            let odbiorcy: Vec<i64> = st.read(|s| {
                let mut v = s.notify.channels.clone();
                v.extend(
                    s.bindings
                        .values()
                        .filter(|b| b.notify)
                        .map(|b| b.channel_id),
                );
                v.sort_unstable();
                v.dedup();
                v
            });
            if odbiorcy.is_empty() {
                anyhow::bail!(
                    "żaden kanał nie jest zaznaczony jako odbiorca — zaznacz „wysyłaj tu \
                     podsumowania bota” przy kanale albo wybierz go na liście powiadomień"
                );
            }
            let Some(tg) = st.tg.read().clone() else {
                anyhow::bail!("usługa Telegrama nie działa — powiadomienia nie mają czym pójść");
            };
            // OSIĄGALNOŚĆ, nie samo zaznaczenie.
            //
            // Wysyłka jest „wyślij i zapomnij": gdy czatu nie ma w mapie znanych
            // rozmów, powiadomienie znika z samym wpisem w logu technicznym.
            // Bez tego sprawdzenia przycisk meldował sukces za wiadomość, która
            // nigdy nie wyszła — a mapa jest PUSTA zaraz po starcie procesu,
            // czyli dokładnie wtedy, gdy ktoś naciska „Wyślij test".
            let (osiagalni, glusi): (Vec<i64>, Vec<i64>) =
                odbiorcy.iter().partition(|id| tg.czy_osiagalny(**id));
            if osiagalni.is_empty() {
                anyhow::bail!(
                    "żaden z {} zaznaczonych kanałów nie jest osiągalny dla tej sesji Telegrama. \
                     Najczęstsza przyczyna: bot dopiero wstał i nie pobrał jeszcze listy rozmów — \
                     otwórz ekran kanałów i spróbuj ponownie.",
                    odbiorcy.len()
                );
            }
            st.notify(
                crate::mailer::MailCategory::Lifecycle,
                "CONDUIT: wiadomość testowa",
                "Jeśli to czytasz, powiadomienia z bota docierają na ten kanał.\n\
                 Tą samą drogą przyjdą otwarcia, zamknięcia, błędy zleceń, \
                 utrata połączenia z MT5 i okresowe podsumowania.",
            );
            st.emit(ui::UiEvent::Toast {
                level: if glusi.is_empty() {
                    "success".into()
                } else {
                    "warn".into()
                },
                title: "Wiadomość testowa wysłana".into(),
                text: if glusi.is_empty() {
                    format!("odbiorców: {}", osiagalni.len())
                } else {
                    // Częściowy sukces MUSI wyglądać inaczej niż pełny —
                    // inaczej użytkownik uzna, że doszło wszędzie.
                    format!(
                        "wysłano na {} z {} kanałów; nieosiągalne dla tej sesji: {}",
                        osiagalni.len(),
                        odbiorcy.len(),
                        glusi
                            .iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            });
            Ok(())
        }

        Command::SetNotify { notify } => {
            // DWIE sekcje, bo synchronizacja rusza też powiązania kanałów.
            // Oznaczenie samych ustawień zostawiłoby panel z nieodświeżonymi
            // zaznaczeniami przy kafelkach — czyli z tym samym rozjazdem,
            // który ta synchronizacja ma likwidować.
            st.update(Sections::two(Section::Settings, Section::Bindings), |s| {
                s.notify = notify.clone();
                zsynchronizuj_powiazania_z_odbiorcami(s);
            });
            persist_smtp(st)?;
            let doc = ChannelsDoc {
                bindings: st.read(|s| s.bindings.clone()),
            };
            st.workspace.save_channels(&doc)?;
            odswiez_powiadamiacz(st);
            Ok(())
        }

        Command::SetDrabinka { drabinka, tryb } => {
            // ══ STRAŻ SILNIKOWA (projekt EA-2c) ══
            //
            // NAJPIERW pytanie „czyja to drabinka", dopiero potem cokolwiek
            // innego. Komenda adresowana do drabinki innego trybu niż bieżący
            // jest ODRZUCANA z wpisem w dzienniku — nigdy przekierowywana,
            // nigdy wykonywana po cichu. Brak adresu = tryb bieżący (kontrakt
            // zera dla panelu sprzed tej wersji).
            let tryb_teraz = st.read(|s| s.mode);
            let cel = ui::KtoraDrabinka::dla(tryb.unwrap_or(tryb_teraz));
            if let Err(powod) = ui::straz_drabinki(tryb_teraz, cel) {
                st.log(
                    "commands",
                    "error",
                    "Odmowa: drabinka spoza bieżącego trybu",
                    format!(
                        "{powod}\n\nNic nie zostało zapisane. Jeśli chcesz zmienić tamtą \
                         drabinkę, przełącz najpierw tryb — wtedy panel pokaże ją i tylko ją."
                    ),
                );
                anyhow::bail!("{powod}");
            }

            // WALIDACJA — cała w [`ui::DrabinkaLancuchow::sprawdz`], żeby
            // każda droga zapisu sprawdzała DOKŁADNIE to samo. Reguły i ich
            // uzasadnienia są opisane przy tamtej funkcji; tutaj zostaje samo
            // wywołanie, bo drugi komplet warunków to drugie źródło prawdy.
            let znane: Vec<String> =
                st.read(|s| s.lancuchy.lista.iter().map(|l| l.nazwa.clone()).collect());
            if let Err(e) = drabinka.sprawdz(&znane) {
                anyhow::bail!("{e}");
            }
            // Sam zapis drabinki NIE przełącza łańcucha — robi to dopiero
            // sprawdzenie BALANCE w pętli handlowej. Inaczej zapis w środku
            // sesji przestawiałby konfigurację natychmiast, bez związku
            // z saldem.
            let ile = drabinka.szczeble.len();
            let wl = drabinka.enabled;
            st.update(Sections::one(Section::Settings), |s| {
                // `biezacy_prog` i `ostatnia_zmiana_ts` prowadzi silnik, nie
                // panel — gdyby panel je nadpisywał, każdy zapis wyglądałby
                // jak świeże przełączenie i histereza traciłaby punkt odniesienia.
                let d = s.drabinka_mut(cel);
                let prog = d.biezacy_prog;
                let kiedy = d.ostatnia_zmiana_ts;
                *d = drabinka.clone();
                d.biezacy_prog = prog;
                d.ostatnia_zmiana_ts = kiedy;
                // Wyłącznik z panelu jest INTENCJĄ użytkownika; skuteczność
                // rozstrzyga izolacja trybów, nie treść komendy.
                d.wlacznik = Some(drabinka.enabled);
                ui::przelicz_izolacje_drabinek(s);
            });
            persist_settings(st)?;
            st.log(
                "commands",
                "success",
                format!(
                    "{} {} ({ile} szczebli)",
                    cel.nazwa(),
                    if wl { "WŁĄCZONA" } else { "wyłączona" }
                ),
                "Progi liczą się po BALANCE, nigdy po equity. Pętla handlowa \
                 sprawdza szczebel co 60 s; przy WŁĄCZENIU synchronizacja \
                 następuje od razu (wpis niżej, jeśli była potrzebna)."
                    .to_string(),
            );

            if wl {
                let balance = st.read(|s| s.stats.balance);
                // PORÓWNUJEMY Z ŁAŃCUCHEM WŁAŚCIWYM DLA TRYBU (projekt EA-2).
                // `SetAktywnyLancuch` niżej zapisze wskaźnik tego samego trybu,
                // więc odczyt musi patrzeć na to samo pole — inaczej w AUTO-EA
                // porównanie wypadałoby wobec cudzego wskazania i drabinka albo
                // synchronizowałaby się w kółko, albo nie ruszyła wcale.
                // …i Z DRABINKI WŁAŚCIWEJ DLA TRYBU (EA-2c). `drabinka_biezaca`
                // jest tą, którą właśnie zapisaliśmy — straż wyżej nie
                // przepuściłaby innej.
                let szczebel = st.read(|s| {
                    (
                        s.drabinka_biezaca()
                            .wybierz(balance)
                            .map(|x| (x.prog_balance, x.lancuch.clone())),
                        s.aktywny_lancuch_nazwa().to_string(),
                    )
                });
                let (szczebel, aktywny) = szczebel;
                if let Some((prog, lancuch)) = szczebel {
                    if lancuch != aktywny {
                        apply(
                            st,
                            &Command::SetAktywnyLancuch {
                                nazwa: lancuch.clone(),
                            },
                        )?;
                        st.update(Sections::one(Section::Settings), |s| {
                            let d = s.drabinka_biezaca_mut();
                            d.biezacy_prog = prog;
                            d.ostatnia_zmiana_ts = crate::now_ms();
                        });
                        st.log(
                            "plan",
                            "success",
                            format!("DRABINKA: synchronizacja szczebla → {lancuch}"),
                            format!(
                                "DRABINKA: synchronizacja szczebla przy włączeniu — \
                                 balance {balance:.2} → próg {prog:.0} → łańcuch {lancuch} \
                                 (aktywny był {aktywny}). Przebudowa silników \
                                 i adopcja koszyków nastąpi w pętli w ciągu ~2 s."
                            ),
                        );
                    }
                }
            }
            Ok(())
        }

        Command::ToggleFavorite { symbol } => {
            st.update(Sections::one(Section::Settings), |s| {
                if let Some(i) = s.favorites.iter().position(|x| x == symbol) {
                    s.favorites.remove(i);
                } else {
                    s.favorites.push(symbol.clone());
                }
            });
            persist_settings(st)?;
            Ok(())
        }

        Command::ClearLogs => {
            st.update(Sections::one(Section::Logs), |s| s.logs.clear());
            Ok(())
        }

        // ---------------- symulacje (stan trzyma serwer) ----------------
        Command::AddSim {
            preset,
            name,
            balance,
            lot,
            mode,
        } => {
            if matches!(mode, Some(ui::TradingMode::Manual)) {
                anyhow::bail!(
                    "tryb MANUAL nie ma w symulacji sensu — wybierz AUTO, AUTO-EA albo AI"
                );
            }
            let id = format!("sim{}", crate::now_ms());
            let mut rec = serde_json::json!({
                "id": id, "name": name, "preset": preset,
                "balance": balance, "startBalance": balance, "equity": balance,
                "lot": lot, "positions": 0, "pendings": 0, "baskets": 0,
                "trades": 0, "winRate": 0, "maxDd": 0,
                "createdAt": crate::now_ms(), "curve": [balance],
            });
            // WŁASNY tryb handlu instancji (patrz `crate::symulacje`).
            // Klucz wchodzi do rekordu TYLKO, gdy użytkownik tryb wybrał —
            // brak klucza = dziedziczenie trybu głównego bota, czyli
            // dokładnie zachowanie każdej instancji sprzed tej zmiany.
            if let Some(m) = mode {
                if let Some(o) = rec.as_object_mut() {
                    o.insert("mode".into(), serde_json::to_value(m)?);
                }
            }
            st.update(Sections::one(Section::Sims), |s| s.sims.push(rec));
            Ok(())
        }
        Command::RemoveSim { id } => {
            st.update(Sections::one(Section::Sims), |s| {
                s.sims
                    .retain(|x| x.get("id").and_then(|v| v.as_str()) != Some(id.as_str()))
            });
            Ok(())
        }
        Command::ResetSim { id } => {
            st.update(Sections::one(Section::Sims), |s| {
                for x in s.sims.iter_mut() {
                    if x.get("id").and_then(|v| v.as_str()) == Some(id.as_str()) {
                        let start = x
                            .get("startBalance")
                            .cloned()
                            .unwrap_or(serde_json::json!(0));
                        if let Some(o) = x.as_object_mut() {
                            o.insert("balance".into(), start.clone());
                            o.insert("equity".into(), start);
                            for k in ["positions", "pendings", "baskets", "trades", "maxDd"] {
                                o.insert(k.into(), serde_json::json!(0));
                            }
                        }
                    }
                }
            });
            Ok(())
        }

        // ---------------- ręczny sygnał ----------------
        // Musi działać w OBU trybach: przy uruchomionym demo wiadomość wpada
        // do wirtualnego brokera, poza demo — do prawdziwego silnika. Parsowanie
        // i pokazanie w panelu dzieje się zawsze, także wtedy, gdy nie ma czym
        // sygnału wykonać (patrz `demo::manual_signal`).
        // CAŁE polecenie idzie dalej, nie wybrane pola. Rozbieranie go tutaj
        // i składanie z powrotem niżej było powodem, dla którego `topic_id`,
        // `msg_id`, `reply_to` i `edit_of` nie miały jak dojechać do silnika.
        Command::SimulateMessage { .. } => crate::demo::manual_signal_scoped(st, cmd, account_session).map(|_| ()),

        // ---------------- handel ----------------
        other => {
            // Przy uruchomionym demo brokerem jest symulator, więc polecenia
            // handlowe idą do jego pętli zamiast do (nieobecnego) MT5.
            if let Some(r) = crate::demo::try_command(st, other) {
                return r;
            }
            let rt = st.runtime.read().clone();
            rt.command_scoped(other, st, account_session)
        }
    }
}

/// Commit only after the live loop verified its current broker/account state.
/// This function does not dispatch again; the runtime owns safe transition checks.
pub fn commit_mode_change(st: &StateHandle, mode: ui::TradingMode) -> Result<()> {
    // DWIE SEKCJE, bo zmiana trybu przestawia też, KTÓRA drabinka jest
    // skuteczna (projekt EA-2c) — a drabinka jedzie w `Settings`.
    // Oznaczenie samego `Mode` zostawiłoby panel z drabinką drugiego
    // trybu na ekranie aż do najbliższej niezwiązanej zmiany ustawień.
    st.update(Sections::two(Section::Mode, Section::Settings), |s| {
        s.mode = mode;
        ui::przelicz_izolacje_drabinek(s);
    });
    persist_settings(st)?;
    let (d_wsp, d_ea) = st.read(|s| (s.drabinka.enabled, s.drabinka_ea.enabled));
    st.log(
        "commands",
        "info",
        format!("Zmiana trybu → {}", mode_label(mode)),
        format!(
            "Drabinki po zmianie: {} — {} · {} — {}. Drabinka NIE-swojego trybu jest \
             bezczynna (jej wyłącznik zostaje zapamiętany i wraca razem z trybem).",
            ui::KtoraDrabinka::Wspolna.nazwa(),
            if d_wsp { "SKUTECZNA" } else { "bezczynna" },
            ui::KtoraDrabinka::Ea.nazwa(),
            if d_ea { "SKUTECZNA" } else { "bezczynna" },
        ),
    );
    // MANUAL nie uruchamia zarządzania pozycją, więc sprzeczności
    // w nim śpią — ostrzegamy dokładnie w chwili, w której zaczynają
    // mieć znaczenie, czyli przy przejściu na AUTO/AI.
    if !matches!(mode, ui::TradingMode::Manual) {
        zglos_niespojnosci(st, &format!("przejście w tryb {}", mode_label(mode)));
}
Ok(())
}

pub fn drabinka_krok(st: &StateHandle, balance: f64) -> Option<String> {
    let (tryb, drabinka, aktywny) = st.read(|s| {
        (
            s.mode,
            s.drabinka_biezaca().clone(),
            s.aktywny_lancuch_nazwa().to_string(),
        )
    });
    let szczebel = drabinka.wybierz(balance)?;
    let (prog, lancuch) = (szczebel.prog_balance, szczebel.lancuch.clone());

    // Szczebel bez zmiany łańcucha → co najwyżej dopisz punkt odniesienia
    // histerezy (backup mógł nie zdążyć przed restartem).
    if lancuch == aktywny {
        if (drabinka.biezacy_prog - prog).abs() > 1e-9 {
            st.update(Sections::one(Section::Settings), |s| {
                s.drabinka_biezaca_mut().biezacy_prog = prog;
            });
        }
        return None;
    }

    // ══ STRAŻ SILNIKOWA ══ Drabinka, którą właśnie przeczytaliśmy, MUSI
    // należeć do trybu, w którym bot pracuje — inaczej za chwilę zapisałaby
    // cudze pole. Dziś jest to niemożliwe z konstrukcji (`drabinka_biezaca`),
    // ale sprawdzenie zostaje: kosztuje jedno porównanie, a jest jedynym
    // miejscem, w którym ta klasa błędu daje się złapać PRZED zapisem.
    if let Err(powod) = ui::straz_drabinki(tryb, ui::KtoraDrabinka::dla(tryb)) {
        st.log("plan", "error", "DRABINKA: krok wstrzymany", powod);
        return None;
    }

    match apply(
        st,
        &Command::SetAktywnyLancuch {
            nazwa: lancuch.clone(),
        },
    ) {
        Ok(()) => {
            st.update(Sections::one(Section::Settings), |s| {
                let d = s.drabinka_biezaca_mut();
                d.biezacy_prog = prog;
                d.ostatnia_zmiana_ts = crate::now_ms();
            });
            let kierunek = if prog >= drabinka.biezacy_prog {
                "≥"
            } else {
                "<"
            };
            let tresc = format!(
                "{}: środki własne {balance:.2} {kierunek} {prog:.0} → łańcuch {lancuch} \
                 (było: {aktywny}). Przebudowa silników i adopcja koszyków nastąpi \
                 w tej samej pętli.",
                ui::KtoraDrabinka::dla(tryb).nazwa()
            );
            st.log(
                "plan",
                "success",
                format!("DRABINKA: → {lancuch}"),
                tresc.clone(),
            );
            st.notify(
                crate::mailer::MailCategory::Lifecycle,
                &format!("DRABINKA: {lancuch}"),
                &tresc,
            );
            Some(lancuch)
        }
        Err(e) => {
            st.log(
                "plan",
                "error",
                "DRABINKA: błąd przełączenia",
                format!(
                    "Nie udało się ustawić łańcucha „{lancuch}” przy balance {balance:.2}: {e}. \
                     Bot gra dalej łańcuchem „{aktywny}”."
                ),
            );
            None
        }
    }
}

/// Łatka ustawień: scalenie + zapis + delta.
pub fn apply_settings_patch(st: &StateHandle, patch: &serde_json::Value) -> Result<()> {
    st.read(|s| settings_map::validate_t100_patch(&s.settings, patch))
        .map_err(anyhow::Error::msg)?;
    let patch = if let Some(jezyk) = patch
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|j| !j.is_empty())
    {
        let jezyk = match jezyk {
            "pl" => "pl",
            "en" => "en",
            inny => {
                anyhow::bail!("nieznany język „{inny}” — dostępne: en, pl");
            }
        };
        st.update(Sections::one(Section::Settings), |s| {
            s.language = jezyk.to_string()
        });
        let mut p = patch.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("language");
        }
        std::borrow::Cow::Owned(serde_json::Value::from(p))
    } else if patch.get("language").is_some() {
        // pusty/nie-string: usuń klucz, nie zapisuj śmiecia
        let mut p = patch.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("language");
        }
        std::borrow::Cow::Owned(serde_json::Value::from(p))
    } else {
        std::borrow::Cow::Borrowed(patch)
    };
    let patch: &serde_json::Value = &patch;

    // ---------- HASŁO MT5: skrzynka podawcza, nie ustawienie ----------
    //
    // Ten sam wzorzec co hasło SMTP (`Command::SetEmail`): pole w panelu
    // służy WYŁĄCZNIE do podania wartości. Ląduje w `secrets.json` (prawa
    // tylko dla właściciela), a z łatki znika, ZANIM cokolwiek trafi do
    // dokumentu ustawień — `settings.json` użytkownik kopiuje między
    // maszynami i wkleja do zgłoszeń, hasło nie ma tam czego szukać.
    // Puste pole znaczy „zostaw stare" — panel nigdy nie dostaje wartości,
    // więc nie miałby czego odesłać.
    let patch = if let Some(haslo) = patch
        .get("mt5_password")
        .and_then(|v| v.as_str())
        .filter(|h| !h.trim().is_empty())
    {
        let mut sek = st.workspace.load_secrets();
        sek.mt5.password = haslo.to_string().into();
        st.workspace.save_secrets(&sek)?;
        st.log(
            "mt5",
            "info",
            "Zapisano hasło rachunku MT5",
            "trafiło do secrets.json (osobny plik, prawa tylko dla właściciela); \
             sidecar dostaje je zmienną środowiskową przy następnym podłączeniu",
        );
        let mut p = patch.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("mt5_password");
        }
        std::borrow::Cow::Owned(p)
    } else if patch.get("mt5_password").is_some() {
        // puste pole — usuń klucz, żeby nie osiadł w dokumencie jako ""
        let mut p = patch.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("mt5_password");
        }
        std::borrow::Cow::Owned(p)
    } else {
        std::borrow::Cow::Borrowed(patch)
    };
    let patch: &serde_json::Value = &patch;

    if let Some(dir) = patch
        .get("alllogs_dir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        crate::alllogs::sprawdz_katalog(std::path::Path::new(dir))?;
    }

    const RACHUNEK_BEZ_SILNIKA: &[&str] = &[
        "mt5_login",
        "mt5_server",
        "mt5_password",
        "przerwa_dobowa_od_h",
        "przerwa_dobowa_do_h",
        "puls_h",
        "alllogs_dir",
        "alert_dd_pct",
        "archive_retention_days",
    ];
    let rusza_silnik = patch
        .as_object()
        .map(|o| {
            o.keys().any(|k| {
                !settings_map::UI_ONLY_KEYS.contains(&k.as_str())
                    && !RACHUNEK_BEZ_SILNIKA.contains(&k.as_str())
            })
        })
        .unwrap_or(false);

    st.update(Sections::one(Section::Settings), |s| {
        // Recheck under the mutation lock: another settings patch may have
        // changed a coupled T-100 bound since the early validation.
        settings_map::validate_t100_patch(&s.settings, patch)?;
        settings_map::merge_patch(&mut s.settings, patch);
        // ręczna zmiana ustawienia zdejmuje etykietę presetu — inaczej panel
        // pokazywałby preset, którym silnik już nie gra (błąd znany z bot.py)
        if rusza_silnik {
            s.preset_id.clear();
        }
        if let Some(l) = settings_map::preset_lot(&s.settings) {
            s.lot = l;
        }
        Ok::<(), String>(())
    }).map_err(anyhow::Error::msg)?;
    persist_settings(st)?;
    // Tylko przy łatce ruszającej SILNIK. Zmiana motywu panelu albo numeru
    // rachunku nie ma jak wprowadzić sprzeczności w zarządzaniu pozycją,
    // a wpis „ustawienia sprzeczne" po przełączeniu motywu byłby szumem.
    if rusza_silnik {
        zglos_niespojnosci(st, "zmiana ustawień");
    }
    Ok(())
}

/// LISTA ODBIORCÓW PODSUMOWAŃ JEST JEDNA — pokazana w dwóch miejscach.
///
/// Panel ma zaznaczenie „wysyłaj tu podsumowania bota" przy kafelku kanału
/// i listę na karcie „Powiadomienia Telegram". To ma być ten sam zbiór:
/// kliknięcie po jednej stronie musi być widoczne po drugiej. Zamiast trzymać
/// dwa stany, które rozjadą się przy pierwszym kliknięciu, po każdej zmianie
/// przepisujemy jedną stronę w drugą.
fn zsynchronizuj_odbiorcow_z_powiazan(s: &mut ui::UiSnapshot) {
    let mut v: Vec<i64> = s
        .bindings
        .values()
        .filter(|b| b.notify)
        .map(|b| b.channel_id)
        .collect();
    v.sort_unstable();
    s.notify.channels = v;
}

/// Kierunek odwrotny: lista z karty powiadomień wyznacza zaznaczenia kafelków.
fn zsynchronizuj_powiazania_z_odbiorcami(s: &mut ui::UiSnapshot) {
    let wybrani: std::collections::HashSet<i64> = s.notify.channels.iter().copied().collect();
    for b in s.bindings.values_mut() {
        b.notify = wybrani.contains(&b.channel_id);
    }
    // Kanał zaznaczony na karcie, który nie ma jeszcze powiązania, musi je
    // dostać — inaczej zaznaczenie zniknęłoby przy najbliższej synchronizacji
    // w drugą stronę.
    for id in wybrani {
        s.bindings
            .entry(id.to_string())
            .or_insert_with(|| ui::ChannelBinding {
                channel_id: id,
                monitored: false,
                notify: true,
                format: String::new(),
                topics: Default::default(),
            });
    }
}

/// Czytelny opis łańcucha do dziennika: co gra na którym formacie i jakie
/// pułapy obowiązują ponad presetami.
fn opis_lancucha(l: &conduit_core::formaty::Lancuchy) -> String {
    opis_lancucha_nazwa(l, &l.aktywny)
}

/// To samo, ale dla ŁAŃCUCHA WSKAZANEGO Z NAZWY — bo od projektu EA-2
/// „aktywny" zależy od trybu i wpis do dziennika ma opisywać ten łańcuch,
/// którego zmiana dotyczyła, a nie ten, który stoi w `lancuchy.aktywny`.
fn opis_lancucha_nazwa(l: &conduit_core::formaty::Lancuchy, nazwa: &str) -> String {
    let Some(a) = l.lista.iter().find(|x| x.nazwa == nazwa) else {
        return "brak aktywnego łańcucha".into();
    };
    let mut s = String::new();
    for (f, p) in &a.presety {
        s.push_str(&format!(
            "{f} → {}
",
            if p.is_empty() {
                "(nie handluje)"
            } else {
                p.as_str()
            }
        ));
    }
    let g = &a.pulapy;
    s.push_str(&format!(
        "
Pułapy łańcucha (0 = bez pułapu): pozycje {} · koszyki {} · loty {:.2} · 
         obsunięcie {:.1} % / {:.2} $ · podłoga equity {:.2} $ · 
         blokada przeciwnych kierunków: {}",
        g.max_pozycji,
        g.max_koszykow,
        g.max_lotow,
        g.max_dd_pct,
        g.max_dd_usd,
        g.podloga_equity_usd,
        if g.blokuj_przeciwne_kierunki {
            "tak"
        } else {
            "nie"
        }
    ));
    s
}

/// BRAMKA SPÓJNOŚCI — jedno wywołanie po każdej zmianie, która może
/// wprowadzić sprzeczność.
///
/// Zasady, których ta funkcja pilnuje:
///
///  * **nigdy nie blokuje** — zwraca `()`, nie `Result`; konfiguracja dziwna,
///    ale świadoma, ma prawo pojechać na rachunek,
///  * **milczy, gdy nie ma o czym mówić** — bramka, która zapala się na
///    czystej konfiguracji, zostaje wyłączona po tygodniu,
///  * **jeden wpis na jedną zmianę**, nie jeden na ostrzeżenie: pięć linijek
///    z rzędu w logu panelu czyta się jak awaria, a to jest lista uwag.
///
/// `powod` mówi, CO wywołało sprawdzenie („wczytanie presetu X"). Bez tego
/// wpis w dzienniku nie daje się powiązać z kliknięciem, które go wywołało.
fn zglos_niespojnosci(st: &StateHandle, powod: &str) {
    // Pułapy sprawdzamy na łańcuchu WŁAŚCIWYM DLA TRYBU (projekt EA-2) —
    // w AUTO-EA `lancuchy.aktywny` bywa czymś innym niż to, czym bot gra.
    let (ustawienia, lancuchy, ea, tryb) = st.read(|s| {
        (
            s.settings.clone(),
            s.lancuchy.clone(),
            s.aktywny_ea.clone(),
            s.mode,
        )
    });
    let mut lista = crate::bramka_spojnosci::sprawdz_dokument(&ustawienia);
    if let Some(a) = ui::lancuch_dla(&lancuchy, &ea, tryb) {
        lista.extend(crate::bramka_spojnosci::sprawdz_pulapy(&a.pulapy));
    }
    if lista.is_empty() {
        return;
    }
    st.log(
        "settings",
        "warn",
        format!(
            "Ustawienia wewnętrznie sprzeczne — {} uwag ({powod})",
            lista.len()
        ),
        format!(
            "To jest OSTRZEŻENIE, nie błąd: nic nie zostało zablokowane ani zmienione.\n\n{}",
            crate::bramka_spojnosci::opis_do_dziennika(&lista)
        ),
    );
}

fn persist_settings(st: &StateHandle) -> Result<()> {
    let doc = st.read(|s| SettingsDoc {
        mode: s.mode,
        preset_id: s.preset_id.clone(),
        lot: s.lot.clone(),
        favorites: s.favorites.clone(),
        language: s.language.clone(),
        settings: s.settings.clone(),
    });
    st.workspace.save_settings(&doc)
}

/// Choosing a strategy is not authorization to switch/enable a broker account.
/// Only follow-terminal mode opts into this guard; OFF retains legacy merging.
/// Explicit SettingsPatch is still the intentional path to change these values.
fn preserve_follow_terminal_account(current: &Value, incoming: &mut Value) {
    if current.get("mt5_follow_terminal_account").and_then(Value::as_bool) != Some(true) {
        return;
    }
    if !incoming.is_object() {
        // A malformed/null preset must not replace the whole settings document.
        *incoming = Value::Object(Default::default());
    }
    let out = incoming.as_object_mut().unwrap();
    let aliases_and_runtime = [
        "server_tz_offset_h", "msg_clock_offset_h", "sim_stops_level", "ai_mode",
        "mt5_follow_terminal_account", "mt5_allow_real_account", "mt5_login",
        "mt5_server", "mt5_password", "mt5_terminal_path", "mt5_python",
        "mt5_symbol", "mt5_magic", "mt5_deviation_points",
    ];
    for key in conduit_core::wielosilnik::POLA_RACHUNKU.iter().copied()
        .chain(aliases_and_runtime) {
        if let Some(value) = current.get(key) {
            out.insert(key.into(), value.clone());
        } else {
            // Missing authorization/identity stays missing. Never import the
            // preset's credentials or infer defaults for an unknown account.
            out.remove(key);
        }
    }
}

fn persist_smtp(st: &StateHandle) -> Result<()> {
    // `redacted()`: nawet gdyby hasło jakimś sposobem trafiło do stanu,
    // nie może wylądować w `smtp.json` — od tego jest `secrets.json`
    let doc = st.read(|s| SmtpDoc {
        email: s.email.redacted(),
        notify: s.notify.clone(),
    });
    st.workspace.save_smtp(&doc)
}

/// Podaje powiadamiaczowi nową konfigurację. Bez tego zmiana ustawień poczty
/// działałaby dopiero po restarcie — czyli w praktyce nigdy nie wtedy,
/// kiedy użytkownik ją testuje.
fn odswiez_powiadamiacz(st: &StateHandle) {
    let Some(n) = st.notifier.read().clone() else {
        return;
    };
    n.reconfigure(crate::notify::setup_from_state(st));
}

fn mode_label(m: ui::TradingMode) -> &'static str {
    match m {
        ui::TradingMode::Manual => "MANUAL",
        ui::TradingMode::Auto => "AUTO",
        ui::TradingMode::AutoEa => "AUTO-EA",
        ui::TradingMode::Ai => "AI",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::mask;
    use serde_json::json;

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-cmd-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        crate::bootstrap(&cfg, crate::default_auth()).unwrap()
    }

    #[test]
    fn mode_change_deferred_runtime_preserves_state_and_disk_until_verified_commit() {
        struct Deferred(std::sync::Mutex<Vec<(ui::TradingMode, Option<String>)>>);
        impl crate::Runtime for Deferred {
            fn command(&self, _: &Command, _: &StateHandle) -> anyhow::Result<()> {
                anyhow::bail!("unexpected generic dispatch")
            }
            fn defer_mode_change(&self, mode: ui::TradingMode, _: &StateHandle,
                account_session: Option<&str>) -> anyhow::Result<bool> {
                self.0.lock().unwrap().push((mode, account_session.map(str::to_owned)));
                Ok(true)
            }
        }
        let st = stan("mode-deferred");
        commit_mode_change(&st, ui::TradingMode::Auto).unwrap();
        let before = st.read(|s| (s.mode, s.settings.clone(), s.drabinka.clone(), s.drabinka_ea.clone()));
        let disk = std::fs::read(st.workspace.settings_path()).unwrap();
        let runtime = std::sync::Arc::new(Deferred(std::sync::Mutex::new(Vec::new())));
        st.set_runtime(runtime.clone());
        apply_scoped(&st, &Command::SetMode { mode: ui::TradingMode::AutoEa }, Some("render-A-17")).unwrap();
        assert_eq!(st.read(|s| (s.mode, s.settings.clone(), s.drabinka.clone(), s.drabinka_ea.clone())), before);
        assert_eq!(std::fs::read(st.workspace.settings_path()).unwrap(), disk);
        assert_eq!(*runtime.0.lock().unwrap(), vec![(ui::TradingMode::AutoEa, Some("render-A-17".into()))]);
        // The live loop alone invokes this after broker checks; no recursive dispatch.
        commit_mode_change(&st, ui::TradingMode::AutoEa).unwrap();
        assert_eq!(st.read(|s| s.mode), ui::TradingMode::AutoEa);
        assert_eq!(runtime.0.lock().unwrap().len(), 1);
        assert_eq!(st.workspace.load_settings().mode, ui::TradingMode::AutoEa);
    }

    #[test]
    fn mode_change_rejected_runtime_never_falls_back_to_offline_commit() {
        struct Rejected;
        impl crate::Runtime for Rejected {
            fn command(&self, _: &Command, _: &StateHandle) -> anyhow::Result<()> { Ok(()) }
            fn defer_mode_change(&self, _: ui::TradingMode, _: &StateHandle,
                _: Option<&str>) -> anyhow::Result<bool> {
                anyhow::bail!("synthetic reconnect with unresolved exposure")
            }
        }
        let st = stan("mode-rejected");
        commit_mode_change(&st, ui::TradingMode::AutoEa).unwrap();
        let before = std::fs::read(st.workspace.settings_path()).unwrap();
        st.set_runtime(std::sync::Arc::new(Rejected));
        assert!(apply(&st, &Command::SetMode { mode: ui::TradingMode::Auto }).is_err());
        assert_eq!(st.read(|s| s.mode), ui::TradingMode::AutoEa);
        assert_eq!(std::fs::read(st.workspace.settings_path()).unwrap(), before);
        // Explicit NoRuntime preserves the existing offline configuration behavior.
        st.set_runtime(std::sync::Arc::new(crate::state::NoRuntime));
        apply(&st, &Command::SetMode { mode: ui::TradingMode::Auto }).unwrap();
        assert_eq!(st.read(|s| s.mode), ui::TradingMode::Auto);
    }

    #[test]
    fn t100_invalid_patch_rejects_before_settings_language_or_disk_mutation() {
        let st = stan("t100-atomic");
        let disk_before = std::fs::read(st.workspace.settings_path()).ok();
        let before = st.read(|s| (s.settings.clone(), s.language.clone(), s.preset_id.clone()));
        assert!(apply_settings_patch(&st, &json!({"language":"pl", "t100":{"enabled":true,"risk_pct":false}})).is_err());
        assert_eq!(st.read(|s| (s.settings.clone(), s.language.clone(), s.preset_id.clone())), before);
        assert_eq!(std::fs::read(st.workspace.settings_path()).ok(), disk_before);
        apply_settings_patch(&st, &json!({"t100":{"enabled":false,"risk_pct":2.0,"portfolio_risk_pct":3.0}})).unwrap();
        apply_settings_patch(&st, &json!({"t100":{"signal_weight":0.0}})).unwrap();
        let now = st.read(|s| settings_map::core_from_ui(&s.settings).t100);
        assert_eq!(now.risk_pct, 2.0);
        assert_eq!(now.portfolio_risk_pct, 3.0);
        assert_eq!(now.signal_weight, 0.0);
        assert!(!now.enabled);
    }

    fn follow_account_fixture() -> Value {
        json!({"mt5_follow_terminal_account":true,"mt5_allow_real_account":false,
            "mt5_login":12345,"mt5_server":"SYNTHETIC-DEMO","mt5_symbol":"XAUUSD",
            "mt5_terminal_path":"C:/synthetic/demo/terminal64.exe","mt5_magic":770077,
            "mt5_autostart":false,"mt5_watchdog":false,"credit_balance_separate":true,
            "close_receipt_reconcile":true,
            "odlicz_kredyt":true,"kredyt_reczny":0.0,
            "server_tz_offset_h":3.0,"msg_clock_offset_h":3.0,
            "sim_stops_level":0.2,"ai_mode":"off"})
    }

    #[test]
    fn account_session_rejects_missing_stale_and_a_b_a_generations() {
        let st=stan("account-scope");
        let cmd=Command::ClosePosition {ticket:7};
        // Legacy OFF is unchanged, including old clients with no envelope.
        assert!(validate_account_session(&st,&cmd,None).is_ok());
        st.update(Sections::all(),|s| {
            s.settings["mt5_follow_terminal_account"]=json!(true);
            s.connection.mt5="connected".into();
            s.connection.account_verified="ok".into();
            s.connection.account_session="A-1".into();
        });
        for token in [None,Some(""),Some("B-2")] {
            assert!(validate_account_session(&st,&cmd,token).is_err());
        }
        assert!(validate_account_session(&st,&cmd,Some("A-1")).is_ok());
        st.update(Sections::all(),|s|s.connection.account_session="B-2".into());
        assert!(validate_account_session(&st,&cmd,Some("A-1")).is_err());
        st.update(Sections::all(),|s|s.connection.account_session="A-3".into());
        // Same account and same ticket after returning A: old A-1 is stale.
        assert!(validate_account_session(&st,&cmd,Some("A-1")).is_err());
        assert!(validate_account_session(&st,&cmd,Some("B-2")).is_err());
        assert!(validate_account_session(&st,&cmd,Some("A-3")).is_ok());
        st.update(Sections::all(),|s|s.connection.mt5="disconnected".into());
        assert!(validate_account_session(&st,&cmd,Some("A-3")).is_err());
        assert!(validate_account_session(&st,&Command::ResumeTrading,Some("A-3")).is_err());
        assert!(validate_account_session(&st,&Command::RearmGuard,Some("A-3")).is_err());
        // Configuration remains accessible while disconnected.
        assert!(validate_account_session(&st,&Command::ResetSettings,None).is_ok());
        apply_settings_patch(&st,&json!({"mt5_allow_real_account":false})).unwrap();
    }

    #[test]
    fn account_session_original_token_reaches_runtime_and_manual_signal() {
        use crate::state::Runtime;
        use std::sync::{Arc,Mutex};
        struct Probe(Arc<Mutex<Vec<(String,Option<String>)>>>);
        impl Runtime for Probe {
            fn command(&self,_:&Command,_:&StateHandle)->anyhow::Result<()> {panic!("unscoped runtime path")}
            fn command_scoped(&self,c:&Command,_:&StateHandle,scope:Option<&str>)->anyhow::Result<()> {
                self.0.lock().unwrap().push((serde_json::to_value(c).unwrap()["cmd"].as_str().unwrap().into(),scope.map(str::to_owned)));
                Ok(())
            }
        }
        let st=stan("account-runtime");let calls=Arc::new(Mutex::new(Vec::new()));
        *st.runtime.write()=Arc::new(Probe(calls.clone()));
        st.update(Sections::all(),|s| {
            s.settings["mt5_follow_terminal_account"]=json!(true);
            s.connection.mt5="connected".into();s.connection.account_verified="ok".into();
            s.connection.account_session="A-3".into();
        });
        let close=Command::ClosePosition {ticket:7};
        assert!(apply(&st,&close).is_err());
        assert!(apply_scoped(&st,&close,Some("B-2")).is_err());
        assert!(calls.lock().unwrap().is_empty());
        apply_scoped(&st,&close,Some("A-3")).unwrap();
        let manual=Command::SimulateMessage {text:"BUY GOLD @ 4000/3995 TP 4010 SL 3990".into(),channel_id:Some(-100),topic_id:None,msg_id:Some(17),reply_to:None,edit_of:None};
        let before=st.read(|s|s.messages.len());
        assert!(crate::demo::manual_signal_scoped(&st,&manual,Some("B-2")).is_err());
        assert_eq!(st.read(|s|s.messages.len()),before,"reject before message insertion");
        apply_scoped(&st,&manual,Some("A-3")).unwrap();
        let got=calls.lock().unwrap();
        assert_eq!(got.len(),2);
        assert_eq!(got[0],("closePosition".into(),Some("A-3".into())));
        assert_eq!(got[1],("simulateMessage".into(),Some("A-3".into())));
    }

    #[test]
    fn follow_terminal_preset_and_reset_preserve_account_authorization() {
        let st=stan("follow-preset");let before=follow_account_fixture();
        st.update(Sections::one(Section::Settings),|s|settings_map::merge_patch(&mut s.settings,&before));
        apply(&st,&Command::ApplyPreset {id:"SYNTHETIC-INLINE".into(),values:Some(json!({
            "mt5_follow_terminal_account":false,"mt5_allow_real_account":true,
            "mt5_login":99999,"mt5_server":"SYNTHETIC-REAL","mt5_symbol":"XAUUSD.s",
            "mt5_terminal_path":"C:/synthetic/real/terminal64.exe","mt5_magic":42,
            "mt5_password":"synthetic-must-not-import","mt5_python":"foreign-python",
            "mt5_autostart":true,"mt5_watchdog":true,"credit_balance_separate":false,
            "close_receipt_reconcile":false,
            "server_tz_offset_h":9.0,"msg_clock_offset_h":9.0,"sim_stops_level":9.0,
            "ai_mode":"on","entry_units":3
        }))}).unwrap();
        let after=st.read(|s|s.settings.clone());
        for (key,want) in before.as_object().unwrap() {assert_eq!(after.get(key),Some(want),"preset replaced {key}");}
        assert_eq!(after["entry_units"],3,"strategy still changes");
        assert!(after.get("mt5_password").is_none());assert!(after.get("mt5_python").is_none());
        apply(&st,&Command::ResetSettings).unwrap();
        let reset=st.read(|s|s.settings.clone());
        for (key,want) in before.as_object().unwrap() {assert_eq!(reset.get(key),Some(want),"reset replaced {key}");}
        assert!(reset.get("mt5_password").is_none());assert!(reset.get("mt5_python").is_none());
        let persisted=st.workspace.load_settings();
        assert_eq!(persisted.settings["mt5_follow_terminal_account"],true);
        assert_eq!(persisted.settings["mt5_allow_real_account"],false);
    }

    #[test]
    fn follow_terminal_guard_preserves_every_canonical_account_field() {
        let mut current=json!({"mt5_follow_terminal_account":true});let mut incoming=json!({});
        for key in conduit_core::wielosilnik::POLA_RACHUNKU {
            current[*key]=json!(123);incoming[*key]=json!(456);
        }
        preserve_follow_terminal_account(&current,&mut incoming);
        for key in conduit_core::wielosilnik::POLA_RACHUNKU {assert_eq!(incoming[*key],123,"{key}");}
        let mut missing=json!({"mt5_allow_real_account":true,"mt5_login":42});
        preserve_follow_terminal_account(&json!({"mt5_follow_terminal_account":true}),&mut missing);
        assert!(missing.get("mt5_allow_real_account").is_none());assert!(missing.get("mt5_login").is_none());
        let mut malformed=Value::Null;preserve_follow_terminal_account(&current,&mut malformed);
        assert_eq!(malformed["mt5_follow_terminal_account"],true);
    }

    #[test]
    fn follow_terminal_off_is_legacy_and_manual_patch_remains_explicit() {
        let before=json!({"mt5_follow_terminal_account":false,"mt5_allow_real_account":false});
        let original=json!({"mt5_follow_terminal_account":true,"mt5_allow_real_account":true,"mt5_login":999});
        let mut incoming=original.clone();preserve_follow_terminal_account(&before,&mut incoming);
        assert_eq!(incoming,original);
        let st=stan("follow-explicit");
        st.update(Sections::one(Section::Settings),|s|settings_map::merge_patch(&mut s.settings,&follow_account_fixture()));
        apply_settings_patch(&st,&json!({"mt5_allow_real_account":true})).unwrap();
        assert_eq!(st.read(|s|s.settings["mt5_allow_real_account"].clone()),true);
    }

    #[test]
    fn zmiana_ustawienia_silnika_zdejmuje_etykiete_presetu() {
        let st = stan("preset-off");
        st.update(Sections::one(Section::Settings), |s| {
            s.preset_id = "GLEBIA".into()
        });
        apply_settings_patch(&st, &json!({ "max_dd_pct": 60 })).unwrap();
        assert_eq!(
            st.read(|s| s.preset_id.clone()),
            "",
            "panel nie może pokazywać presetu, którym silnik już nie gra"
        );
    }

    /// BRAMKA SPÓJNOŚCI OSTRZEGA, NIGDY NIE BLOKUJE.
    ///
    /// To jest cała umowa tej funkcji i jedyny sposób, żeby jej pilnować:
    /// zapis konfiguracji sprzecznej ma się UDAĆ, wartość ma wylądować
    /// w dokumencie nietknięta, a w logu ma stanąć wpis poziomu `warn`.
    /// Gdyby kiedyś ktoś zamienił ostrzeżenie na `bail!`, ten test zapali się
    /// zanim właściciel zobaczy odmowę zapisu własnych ustawień.
    #[test]
    fn bramka_spojnosci_ostrzega_ale_nie_blokuje_zapisu() {
        let st = stan("spojnosc-warn");
        apply_settings_patch(
            &st,
            &json!({ "expo_cap_pct": 300.0, "expo_cap_close": false }),
        )
        .expect("konfiguracja dziwna, ale świadoma, ma prawo się zapisać");

        assert_eq!(
            st.read(|s| s.settings.get("expo_cap_pct").and_then(|v| v.as_f64())),
            Some(300.0),
            "bramka nie ma prawa zmienić ani jednej wartości"
        );

        let ostrzezenie = st.read(|s| {
            s.logs
                .iter()
                .find(|l| l.level == "warn" && l.title.contains("sprzeczne"))
                .cloned()
        });
        let w = ostrzezenie.expect("brak wpisu ostrzegawczego w dzienniku");
        assert!(
            w.content.contains("S3-straz-nic-nie-zamyka"),
            "{}",
            w.content
        );
        assert!(
            w.content.contains("OSTRZEŻENIE, nie błąd"),
            "wpis musi mówić wprost, że nic nie zostało zablokowane"
        );
    }

    /// Konfiguracja spójna nie może zostawić ANI JEDNEGO wpisu ostrzegawczego.
    /// Bramka, która zapala się na czystych ustawieniach, zostanie wyłączona
    /// po tygodniu i nie złapie już niczego.
    #[test]
    fn bramka_spojnosci_milczy_na_zdrowej_konfiguracji() {
        let st = stan("spojnosc-cisza");
        apply_settings_patch(&st, &json!({ "max_dd_pct": 60 })).unwrap();
        let ile = st.read(|s| {
            s.logs
                .iter()
                .filter(|l| l.level == "warn" && l.title.contains("sprzeczne"))
                .count()
        });
        assert_eq!(ile, 0);
    }

    #[test]
    fn zmiana_motywu_nie_zdejmuje_etykiety_presetu() {
        // REGRESJA: przełącznik motywu leci tą samą drogą co każde inne
        // ustawienie. Bez rozróżnienia „co rusza silnik" przejście na jasny
        // motyw kasowało informację o tym, którym presetem gra bot.
        let st = stan("preset-motyw");
        st.update(Sections::one(Section::Settings), |s| {
            s.preset_id = "GLEBIA".into()
        });

        apply_settings_patch(&st, &json!({ "ui_theme": "light" })).unwrap();
        assert_eq!(st.read(|s| s.preset_id.clone()), "GLEBIA");

        apply_settings_patch(&st, &json!({ "ui_palette": "matrix" })).unwrap();
        assert_eq!(st.read(|s| s.preset_id.clone()), "GLEBIA");

        // ale mieszana łatka (motyw + ustawienie silnika) JUŻ zdejmuje
        apply_settings_patch(&st, &json!({ "ui_theme": "dark", "max_dd_pct": 10 })).unwrap();
        assert_eq!(st.read(|s| s.preset_id.clone()), "");
    }

    #[test]
    fn wyglad_przezywa_restart_bo_lezy_w_settings_json() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-wyglad-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir.clone(),
            ..Default::default()
        };
        {
            let st = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
            apply_settings_patch(&st, &json!({ "ui_theme": "light", "ui_palette": "matrix" }))
                .unwrap();
        }
        // nowy proces, ten sam katalog
        let st2 = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
        assert_eq!(st2.read(|s| s.settings["ui_theme"].clone()), json!("light"));
        assert_eq!(
            st2.read(|s| s.settings["ui_palette"].clone()),
            json!("matrix")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn haslo_smtp_idzie_do_secrets_a_nie_do_smtp_json() {
        let st = stan("haslo");
        let mut e = ui::EmailConfig {
            host: "smtp.example.com".into(),
            ..Default::default()
        };
        e.user = "bot@example.com".into();
        e.pass = "tajne-haslo-aplikacji".into();
        apply(&st, &Command::SetEmail { email: e }).unwrap();

        // 1. hasło NIE zostaje w stanie
        assert_eq!(st.read(|s| s.email.pass.clone()), "");

        // 2. hasła NIE MA w smtp.json
        let smtp_raw = std::fs::read_to_string(st.workspace.smtp_path()).unwrap();
        assert!(
            !smtp_raw.contains("tajne-haslo"),
            "hasło wyciekło do smtp.json: {smtp_raw}"
        );

        // 3. hasło JEST w secrets.json i da się je odczytać
        assert_eq!(
            st.workspace.load_secrets().smtp.password.as_str(),
            "tajne-haslo-aplikacji"
        );

        // 4. migawka wysyłana do przeglądarki go nie niesie
        let snap = serde_json::to_string(&st.snapshot()).unwrap();
        assert!(!snap.contains("tajne-haslo"), "hasło wyciekło do migawki");
    }

    #[test]
    fn puste_haslo_znaczy_zostaw_stare() {
        let st = stan("haslo-puste");
        let baza = ui::EmailConfig {
            host: "smtp.example.com".into(),
            ..Default::default()
        };

        apply(
            &st,
            &Command::SetEmail {
                email: ui::EmailConfig {
                    pass: "pierwsze".into(),
                    ..baza.clone()
                },
            },
        )
        .unwrap();
        assert_eq!(
            st.workspace.load_secrets().smtp.password.as_str(),
            "pierwsze"
        );

        // zapis bez hasła (UI nigdy go nie dostaje) nie może go skasować
        apply(
            &st,
            &Command::SetEmail {
                email: ui::EmailConfig {
                    to: "recipient@example.com".into(),
                    ..baza.clone()
                },
            },
        )
        .unwrap();
        assert_eq!(
            st.workspace.load_secrets().smtp.password.as_str(),
            "pierwsze"
        );
        assert_eq!(st.read(|s| s.email.to.clone()), "recipient@example.com");

        // a jawna zmiana działa
        apply(
            &st,
            &Command::SetEmail {
                email: ui::EmailConfig {
                    pass: "drugie".into(),
                    ..baza
                },
            },
        )
        .unwrap();
        assert_eq!(st.workspace.load_secrets().smtp.password.as_str(), "drugie");
    }

    #[test]
    fn przywrocenie_domyslnych_nie_gasi_motywu() {
        // „Przywróć domyślne" dotyczy USTAWIEŃ SILNIKA. Zgaszenie przy okazji
        // motywu wygląda jak awaria panelu, a nie jak reset konfiguracji.
        let st = stan("reset-motyw");
        apply_settings_patch(
            &st,
            &json!({ "ui_theme": "light", "ui_palette": "matrix", "max_dd_pct": 60 }),
        )
        .unwrap();
        apply(&st, &Command::ResetSettings).unwrap();

        assert_eq!(st.read(|s| s.settings["ui_theme"].clone()), json!("light"));
        assert_eq!(
            st.read(|s| s.settings["ui_palette"].clone()),
            json!("matrix")
        );
        // ale ustawienia silnika wróciły do domyślnych
        assert_eq!(
            st.read(|s| s.settings["max_dd_pct"].clone()),
            json!(conduit_core::Settings::default().max_dd_pct)
        );
    }

    #[test]
    fn kategorie_i_dlawienie_przezywaja_restart() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-poczta-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir.clone(),
            ..Default::default()
        };
        {
            let st = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
            let e = ui::EmailConfig {
                enabled: true,
                to: "one@example.com, two@example.com".into(),
                host: "smtp.example.com".into(),
                port: 465,
                security: crate::mailer::MailSecurity::Ssl,
                categories: crate::mailer::MailCategories {
                    summary: true,
                    drawdown: false,
                    ..Default::default()
                },
                throttle: crate::mailer::ThrottleConfig {
                    window_min: 25.0,
                    max_per_hour: 4,
                },
                ..Default::default()
            };
            apply(&st, &Command::SetEmail { email: e }).unwrap();
        }
        let st2 = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
        let e = st2.read(|s| s.email.clone());
        assert!(e.enabled);
        assert_eq!(e.port, 465);
        assert_eq!(e.security, crate::mailer::MailSecurity::Ssl);
        assert!(e.categories.summary);
        assert!(!e.categories.drawdown);
        assert_eq!(e.throttle.window_min, 25.0);
        assert_eq!(e.throttle.max_per_hour, 4);
        // i odbiorcy rozbijają się poprawnie
        assert_eq!(crate::mailer::parse_recipients(&e.to).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mail_testowy_bez_uruchomionej_poczty_konczy_sie_bledem_a_nie_cisza() {
        let st = stan("test-bez");
        let e = apply(&st, &Command::SendTestEmail).unwrap_err();
        assert!(e.to_string().contains("nie są uruchomione"), "{e}");
    }

    #[test]
    fn maska_sekretu_nie_ujawnia_tresci() {
        // ta sama funkcja maskuje api_hash i hasło SMTP w logach
        let m = mask("tajne-haslo-aplikacji");
        assert!(!m.contains("tajne"));
        assert!(!m.contains("haslo"));
    }


    /// KONTRAKT ZERA: `addSim` bez trybu tworzy rekord BEZ klucza `mode` —
    /// czyli dokładnie taki, jaki tworzył przed zmianą — a `symulacje::
    /// tryb_instancji` czyta go jako dziedziczenie trybu głównego bota.
    #[test]
    fn add_sim_bez_trybu_tworzy_rekord_dziedziczacy() {
        let st = stan("sim-dziedziczenie");
        apply(
            &st,
            &Command::AddSim {
                preset: "HYPER-2".into(),
                name: "bazowa".into(),
                balance: 200.0,
                lot: 0.01,
                mode: None,
            },
        )
        .unwrap();

        let rec = st.read(|s| s.sims[0].clone());
        assert!(
            rec.get("mode").is_none(),
            "brak wyboru = brak klucza, nie null: {rec}"
        );
        assert_eq!(
            crate::symulacje::tryb_instancji(&rec, ui::TradingMode::AutoEa),
            ui::TradingMode::AutoEa
        );
        assert_eq!(
            crate::symulacje::tryb_instancji(&rec, ui::TradingMode::Auto),
            ui::TradingMode::Auto
        );
    }

    #[test]
    fn add_sim_z_trybem_nadpisuje_tryb_glowny() {
        let st = stan("sim-tryb");
        st.update(Sections::one(Section::Mode), |s| {
            s.mode = ui::TradingMode::AutoEa
        });
        apply(
            &st,
            &Command::AddSim {
                preset: "HYPER-2".into(),
                name: "zwykly-auto".into(),
                balance: 200.0,
                lot: 0.01,
                mode: Some(ui::TradingMode::Auto),
            },
        )
        .unwrap();
        apply(
            &st,
            &Command::AddSim {
                preset: "HYPER-2".into(),
                name: "potwor".into(),
                balance: 200.0,
                lot: 0.01,
                mode: Some(ui::TradingMode::AutoEa),
            },
        )
        .unwrap();

        let glowny = st.read(|s| s.mode);
        let (auto, ea) = st.read(|s| (s.sims[0].clone(), s.sims[1].clone()));
        assert_eq!(auto["mode"], serde_json::json!("AUTO"));
        assert_eq!(ea["mode"], serde_json::json!("AUTO-EA"));
        assert!(
            !crate::symulacje::auto_ea_instancji(&auto, glowny),
            "główny AUTO-EA nie może zarazić instancji AUTO"
        );
        assert!(crate::symulacje::auto_ea_instancji(&ea, glowny));
    }

    #[test]
    fn add_sim_odrzuca_manual() {
        let st = stan("sim-manual");
        let e = apply(
            &st,
            &Command::AddSim {
                preset: "HYPER-2".into(),
                name: "".into(),
                balance: 200.0,
                lot: 0.01,
                mode: Some(ui::TradingMode::Manual),
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("MANUAL"), "{e}");
        assert_eq!(
            st.read(|s| s.sims.len()),
            0,
            "po odmowie nie ma prawa być rekordu"
        );
    }
}

#[cfg(test)]
mod testy_synchronizacji_drabinki {
    use super::*;

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-drab-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        crate::bootstrap(&cfg, crate::default_auth()).unwrap()
    }

    #[test]
    fn wlaczenie_drabinki_synchronizuje_szczebel_natychmiast() {
        let st = stan("sync");

        let mut d = crate::ui::DrabinkaLancuchow::default();
        d.enabled = true; // `wybierz` zwraca None dla drabinki wyłączonej
        let (prog0, szczebel0) = {
            let s0 = d
                .wybierz(300.0)
                .expect("domyślna drabinka musi mieć szczebel bazowy");
            (s0.prog_balance, s0.lancuch.clone())
        };

        // Łańcuch startowy MUSI różnić się od szczebla — bez rozjazdu nie ma
        // czego synchronizować i test przechodziłby, niczego nie sprawdzając.
        // Wybieramy pierwszy ZNANY, który nie jest koroną — znowu bez literału.
        let startowy = st.read(|s| {
            s.lancuchy
                .lista
                .iter()
                .map(|l| l.nazwa.clone())
                .find(|n| *n != szczebel0)
                .expect("stan startowy musi znać co najmniej dwa łańcuchy")
        });
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = startowy.clone();
            s.stats.balance = 300.0;
        });
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: d,
                tryb: None,
            },
        )
        .unwrap();

        assert_eq!(
            st.read(|s| s.lancuchy.aktywny.clone()),
            szczebel0,
            "balance 300 → szczebel {szczebel0} MUSI zsynchronizować się od razu \
             (aktywny na starcie: {startowy})"
        );
        assert_eq!(st.read(|s| s.drabinka.biezacy_prog), prog0);
        let jest_wpis = st.read(|s| {
            s.logs
                .iter()
                .any(|l| l.title.contains("synchronizacja szczebla"))
        });
        assert!(
            jest_wpis,
            "wpis „DRABINKA: synchronizacja szczebla” musi być w logu"
        );
    }

    /// DROGA PRODUKCYJNA ODRZUCA ZŁĄ DRABINKĘ — i to jest test na tej samej
    /// funkcji, którą woła gniazdo (`apply`), a nie na kopii walidacji.
    ///
    /// Panel ma swój bliźniaczy komplet reguł, ale panel nie jest jedynym
    /// klientem gniazda: każdy skrypt może wysłać `setDrabinka`. Cicha zgoda
    /// na drabinkę z powtórzonym progiem znaczy bota, który przy tym saldzie
    /// wybiera łańcuch zależnie od kolejności sortowania.
    #[test]
    fn zla_drabinka_jest_odrzucana_przez_apply_z_komunikatem() {
        let st = stan("zla-drabinka");
        let sz = |p: f64, l: &str| crate::ui::SzczebelDrabinki {
            prog_balance: p,
            lancuch: l.into(),
        };
        let z = |szczeble: Vec<crate::ui::SzczebelDrabinki>| crate::ui::DrabinkaLancuchow {
            enabled: true,
            wlacznik: Some(true),
            szczeble,
            histereza_pct: 2.0,
            biezacy_prog: -1.0,
            ostatnia_zmiana_ts: 0,
        };
        let przed = st.read(|s| s.drabinka.clone());

        // duplikat progu
        let e = apply(
            &st,
            &Command::SetDrabinka {
                drabinka: z(vec![
                    sz(0.0, "ZENONLY5"),
                    sz(500.0, "ZENONLY3"),
                    sz(500.0, "SENTINEL-0"),
                ]),
                tryb: None,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("ten sam próg"),
            "komunikat ma nazwać duplikat: {e}"
        );

        // próg mniejszy od poprzedniego
        let e = apply(
            &st,
            &Command::SetDrabinka {
                drabinka: z(vec![
                    sz(0.0, "ZENONLY5"),
                    sz(900.0, "ZENONLY3"),
                    sz(400.0, "SENTINEL-0"),
                ]),
                tryb: None,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("MNIEJSZY"), "komunikat ma nazwać kolejność: {e}");

        // brak szczebla bazowego
        let e = apply(
            &st,
            &Command::SetDrabinka {
                drabinka: z(vec![sz(500.0, "ZENONLY5"), sz(900.0, "ZENONLY3")]),
                tryb: None,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("BAZOWEGO"), "komunikat ma nazwać brak zera: {e}");

        assert_eq!(
            st.read(|s| s.drabinka.clone()),
            przed,
            "odrzucony zapis nie ma prawa nic zmienić"
        );

        // DOWOLNA liczba szczebli przechodzi — siedem, bez limitu
        let dobra = z(vec![
            sz(0.0, "ZENONLY5"),
            sz(300.0, "ZENONLY3"),
            sz(500.0, "SENTINEL-0"),
            sz(800.0, "SENTINEL-0A"),
            sz(1200.0, "ZENONLY3"),
            sz(1800.0, "SENTINEL-0"),
            sz(2500.0, "SENTINEL-0A"),
        ]);
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: dobra,
                tryb: None,
            },
        )
        .unwrap();
        assert_eq!(st.read(|s| s.drabinka.szczeble.len()), 7);
    }

    /// Włączenie drabinki, gdy aktywny łańcuch JUŻ odpowiada szczeblowi —
    /// zero przełączeń, zero wpisów o synchronizacji (nic się nie stało).
    #[test]
    fn wlaczenie_bez_rozjazdu_niczego_nie_przelacza() {
        let st = stan("norozjazd");

        // Ta sama zasada co w teście wyżej: nazwa szczebla pochodzi z `Default`,
        // a nie z literału. Tu ustawiamy aktywny łańcuch NA szczebel, żeby
        // rozjazdu nie było — i to jest cały sens tego testu.
        let mut d = crate::ui::DrabinkaLancuchow::default();
        d.enabled = true; // `wybierz` zwraca None dla drabinki wyłączonej
        let szczebel = d
            .wybierz(300.0)
            .expect("domyślna drabinka musi mieć szczebel bazowy")
            .lancuch
            .clone();

        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = szczebel.clone();
            s.stats.balance = 300.0;
        });
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: d,
                tryb: None,
            },
        )
        .unwrap();
        assert_eq!(st.read(|s| s.lancuchy.aktywny.clone()), szczebel);
        let wpisow = st.read(|s| {
            s.logs
                .iter()
                .filter(|l| l.title.contains("synchronizacja szczebla"))
                .count()
        });
        assert_eq!(wpisow, 0, "bez rozjazdu nie ma czego synchronizować");
    }
}

#[cfg(test)]
mod testy_jezyka {
    use super::*;
    use serde_json::json;

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-jezyk-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        crate::bootstrap(&cfg, crate::default_auth()).unwrap()
    }

    #[test]
    fn jezyk_zapisuje_sie_przezywa_restart_i_nie_rusza_etykiety() {
        let st = stan("pl");
        st.update(Sections::one(Section::Settings), |s| {
            s.preset_id = "HYPER-2".into()
        });

        apply_settings_patch(&st, &json!({ "language": "pl" })).unwrap();
        assert_eq!(st.read(|s| s.language.clone()), "pl");
        assert_eq!(
            st.read(|s| s.preset_id.clone()),
            "HYPER-2",
            "zmiana języka NIE MA PRAWA zdjąć etykiety presetu"
        );
        // klucz NIE osiadł w settings{} — jest polem głównym dokumentu
        assert!(st.read(|s| s.settings.get("language").is_none()));

        // restart: nowy stan z tego samego workspace czyta zapisany język
        let doc = st.workspace.load_settings();
        assert_eq!(
            doc.language, "pl",
            "language musi być w settings.json na poziomie głównym"
        );
        assert_eq!(doc.preset_id, "HYPER-2");
    }

    /// Nieznany język = odmowa (ack z błędem), stan bez zmian — panel nie
    /// może wstać w języku, którego nie ma w słownikach.
    #[test]
    fn nieznany_jezyk_jest_odrzucany() {
        let st = stan("zly");
        let e = apply_settings_patch(&st, &json!({ "language": "de" }));
        assert!(e.is_err());
        assert_eq!(st.read(|s| s.language.clone()), "en");
    }
}

/* ============================================================
EA-2 — OSOBNY AKTYWNY ŁAŃCUCH DLA TRYBU AUTO-EA

Wskaźnik `aktywny_ea` żyje w `lancuchy.json` obok `aktywny`
i obowiązuje WYŁĄCZNIE w trybie AUTO-EA. Testy pilnują trzech
rzeczy, na których stoi cała zmiana:

  * KONTRAKT ZERA — plik bez pola wczytuje się i zachowuje jak
    dotąd (AUTO-EA gra tym, czym grało),
  * FALLBACK — wskazanie w pustkę (łańcuch skasowany) wraca na
    wspólne `aktywny`, zamiast zostawiać bota bez składu,
  * IZOLACJA — przełączenie w AUTO-EA nie rusza `aktywny`,
    a przełączenie w AUTO nie rusza `aktywny_ea`.
============================================================ */
#[cfg(test)]
mod testy_lancucha_ea {
    use super::*;

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-ea2-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        crate::bootstrap(&cfg, crate::default_auth()).unwrap()
    }

    /// Dwie nazwy z listy wbudowanej — bez literałów, żeby test nie umierał
    /// przy każdej zmianie korony (ta klasa błędu zjadła już test drabinki).
    fn dwa_lancuchy(st: &StateHandle) -> (String, String) {
        st.read(|s| {
            let a = s.lancuchy.aktywny.clone();
            let b = s
                .lancuchy
                .lista
                .iter()
                .map(|l| l.nazwa.clone())
                .find(|n| *n != a)
                .expect("stan startowy musi znać co najmniej dwa łańcuchy");
            (a, b)
        })
    }

    /// KONTRAKT ZERA: `lancuchy.json` sprzed tej wersji nie ma pola
    /// `aktywnyEa`. Musi wczytać się bez ostrzeżenia, dać pusty wskaźnik
    /// i zachowywać się w AUTO-EA dokładnie jak w AUTO.
    #[test]
    fn stary_plik_bez_pola_daje_pusty_wskaznik_i_fallback() {
        let st = stan("stary");
        let (aktywny, _) = dwa_lancuchy(&st);

        // dokładnie taki plik, jaki pisała poprzednia wersja
        std::fs::write(
            st.workspace.lancuchy_path(),
            serde_json::to_vec(&serde_json::json!({ "aktywny": aktywny, "lista": [] })).unwrap(),
        )
        .unwrap();

        assert_eq!(
            st.workspace.load_aktywny_ea(),
            "",
            "brak klucza = pusty wskaźnik"
        );
        let z = st.workspace.load_lancuchy();
        assert_eq!(
            ui::aktywny_dla(&z, "", ui::TradingMode::AutoEa),
            aktywny,
            "AUTO-EA bez własnego wskazania gra tym samym, co reszta trybów"
        );
        assert_eq!(ui::aktywny_dla(&z, "", ui::TradingMode::Auto), aktywny);
    }

    /// Wskaźnik pokazujący na łańcuch SKASOWANY z listy nie ma prawa zostawić
    /// AUTO-EA bez składu — wraca na wspólne `aktywny`.
    #[test]
    fn wskazanie_w_pustke_wraca_na_aktywny() {
        let st = stan("pustka");
        let z = st.read(|s| s.lancuchy.clone());
        assert_eq!(
            ui::aktywny_dla(&z, "LANCUCH-KTOREGO-NIE-MA", ui::TradingMode::AutoEa),
            z.aktywny,
            "nazwa spoza listy = fallback, nie cisza"
        );
        assert!(ui::lancuch_dla(&z, "LANCUCH-KTOREGO-NIE-MA", ui::TradingMode::AutoEa).is_some());
    }

    /// Pozostałe tryby NIE WIDZĄ wskaźnika EA — nawet gdy jest poprawny.
    #[test]
    fn tryby_nie_ea_ignoruja_wskaznik() {
        let st = stan("ignor");
        let (aktywny, inny) = dwa_lancuchy(&st);
        let z = st.read(|s| s.lancuchy.clone());
        for tryb in [
            ui::TradingMode::Auto,
            ui::TradingMode::Manual,
            ui::TradingMode::Ai,
        ] {
            assert_eq!(
                ui::aktywny_dla(&z, &inny, tryb),
                aktywny,
                "tryb {tryb:?} nie ma prawa czytać wskaźnika AUTO-EA"
            );
        }
        assert_eq!(ui::aktywny_dla(&z, &inny, ui::TradingMode::AutoEa), inny);
    }

    /// IZOLACJA PÓL — sedno projektu EA-2. Przełączenie łańcucha w AUTO-EA
    /// zapisuje `aktywny_ea` i NIE RUSZA `aktywny`; w AUTO jest odwrotnie.
    /// Bez tego przełączanie trybu tam i z powrotem przepisywałoby skład
    /// drugiej strony po cichu.
    #[test]
    fn zmiana_w_auto_ea_nie_rusza_pola_aktywny() {
        let st = stan("izolacja");
        let (aktywny, inny) = dwa_lancuchy(&st);

        st.update(Sections::one(Section::Mode), |s| {
            s.mode = ui::TradingMode::AutoEa
        });
        apply(
            &st,
            &Command::SetAktywnyLancuch {
                nazwa: inny.clone(),
            },
        )
        .unwrap();

        let (a, ea) = st.read(|s| (s.lancuchy.aktywny.clone(), s.aktywny_ea.clone()));
        assert_eq!(
            a, aktywny,
            "AUTO-EA NIE MA PRAWA ruszyć wspólnego `aktywny`"
        );
        assert_eq!(ea, inny);
        // i to samo NA DYSKU — inaczej izolacja żyje tylko do restartu
        assert_eq!(st.workspace.load_aktywny_ea(), inny);
        assert_eq!(st.workspace.load_lancuchy().aktywny, aktywny);

        // powrót do AUTO: wskaźnik EA zostaje nietknięty, zmienia się `aktywny`
        st.update(Sections::one(Section::Mode), |s| {
            s.mode = ui::TradingMode::Auto
        });
        apply(
            &st,
            &Command::SetAktywnyLancuch {
                nazwa: aktywny.clone(),
            },
        )
        .unwrap();
        let (a2, ea2) = st.read(|s| (s.lancuchy.aktywny.clone(), s.aktywny_ea.clone()));
        assert_eq!(a2, aktywny);
        assert_eq!(ea2, inny, "AUTO nie ma prawa skasować wskazania warstwy EA");
    }

    #[test]
    fn set_lancuchy_bez_pola_nie_kasuje_wskaznika() {
        let st = stan("setall");
        let (aktywny, inny) = dwa_lancuchy(&st);
        st.update(Sections::one(Section::Settings), |s| {
            s.aktywny_ea = inny.clone()
        });

        let z = st.read(|s| s.lancuchy.clone());
        // BRAK pola wolno wysłać z każdego trybu — to jest „nie ruszaj".
        apply(
            &st,
            &Command::SetLancuchy {
                lancuchy: z.clone(),
                aktywny_ea: None,
            },
        )
        .unwrap();
        assert_eq!(
            st.read(|s| s.aktywny_ea.clone()),
            inny,
            "brak pola = nie ruszaj"
        );

        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::AutoEa,
            },
        )
        .unwrap();
        apply(
            &st,
            &Command::SetLancuchy {
                lancuchy: z.clone(),
                aktywny_ea: Some(String::new()),
            },
        )
        .unwrap();
        assert_eq!(
            st.read(|s| s.aktywny_ea.clone()),
            "",
            "pusty = skasuj wskazanie"
        );
        assert_eq!(st.read(|s| s.lancuchy.aktywny.clone()), aktywny);
        assert_eq!(
            st.workspace.load_aktywny_ea(),
            "",
            "pusty wskaźnik NIE zostaje w pliku jako klucz bez znaczenia"
        );

        // wskazanie w pustkę jest ODMAWIANE, a nie zapisywane po cichu
        let e = apply(
            &st,
            &Command::SetLancuchy {
                lancuchy: z,
                aktywny_ea: Some("NIE-MA-TAKIEGO".into()),
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("NIE-MA-TAKIEGO"), "{e}");
    }

    /// Wskaźnik przeżywa RESTART: zapis → nowy `bootstrap` z tego samego
    /// katalogu → ta sama odpowiedź. Bez tego cała izolacja żyje do wyłączenia.
    #[test]
    fn wskaznik_przezywa_restart() {
        let st = stan("restart");
        let (_, inny) = dwa_lancuchy(&st);
        st.update(Sections::one(Section::Mode), |s| {
            s.mode = ui::TradingMode::AutoEa
        });
        apply(
            &st,
            &Command::SetAktywnyLancuch {
                nazwa: inny.clone(),
            },
        )
        .unwrap();

        let cfg = crate::ServerConfig {
            workspace: st.workspace.root.clone(),
            ..Default::default()
        };
        let st2 = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
        assert_eq!(st2.read(|s| s.aktywny_ea.clone()), inny);
    }
}


#[cfg(test)]
mod testy_izolacji_drabinek {
    use super::*;

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-ea2c-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        crate::bootstrap(&cfg, crate::default_auth()).unwrap()
    }

    /// Trzy nazwy Z LISTY WBUDOWANEJ — bez literałów, żeby test nie umierał
    /// przy każdej zmianie korony.
    fn trzy_lancuchy(st: &StateHandle) -> (String, String, String) {
        st.read(|s| {
            let a = s.lancuchy.aktywny.clone();
            let mut inne = s
                .lancuchy
                .lista
                .iter()
                .map(|l| l.nazwa.clone())
                .filter(|n| *n != a);
            let b = inne
                .next()
                .expect("lista musi znać co najmniej trzy łańcuchy");
            let c = inne
                .next()
                .expect("lista musi znać co najmniej trzy łańcuchy");
            (a, b, c)
        })
    }

    fn drabinka(szczeble: &[(f64, &str)]) -> ui::DrabinkaLancuchow {
        ui::DrabinkaLancuchow {
            enabled: true,
            wlacznik: Some(true),
            szczeble: szczeble
                .iter()
                .map(|(p, l)| ui::SzczebelDrabinki {
                    prog_balance: *p,
                    lancuch: (*l).into(),
                })
                .collect(),
            histereza_pct: 2.0,
            biezacy_prog: -1.0,
            ostatnia_zmiana_ts: 0,
        }
    }

    #[test]
    fn wlaczenie_drabinki_w_auto_ea_nie_rusza_aktywnego() {
        let st = stan("a-ea");
        let (aktywny, cel, _) = trzy_lancuchy(&st);
        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::AutoEa,
            },
        )
        .unwrap();

        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &cel)]),
                tryb: Some(ui::TradingMode::AutoEa),
            },
        )
        .unwrap();

        let (a, ea, wsp_szczebli, ea_szczebli) = st.read(|s| {
            (
                s.lancuchy.aktywny.clone(),
                s.aktywny_ea.clone(),
                s.drabinka.szczeble.len(),
                s.drabinka_ea.szczeble.len(),
            )
        });
        assert_eq!(
            a, aktywny,
            "drabinka trybu AUTO-EA NIE MA PRAWA ruszyć wspólnego `aktywny`"
        );
        assert_eq!(ea, cel, "…a swój wskaźnik ma przestawić");
        assert_eq!(ea_szczebli, 1, "zapis trafił do drabinki EA");
        assert_eq!(
            wsp_szczebli,
            ui::DrabinkaLancuchow::default().szczeble.len(),
            "drabinka trybu AUTO została nietknięta"
        );

        // NA DYSKU — to samo zdanie, tylko sprawdzalne po restarcie
        assert_eq!(st.workspace.load_lancuchy().aktywny, aktywny);
        assert_eq!(st.workspace.load_aktywny_ea(), cel);
    }

    /// (b) …i odwrotnie: włączenie drabinki w AUTO nie dotyka `aktywnyEa`.
    #[test]
    fn wlaczenie_drabinki_w_auto_nie_rusza_wskaznika_ea() {
        let st = stan("b-auto");
        let (_, cel, skarb) = trzy_lancuchy(&st);
        // wskazanie warstwy EA ustawione ŚWIADOMIE wcześniej — ma przetrwać
        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::AutoEa,
            },
        )
        .unwrap();
        apply(
            &st,
            &Command::SetAktywnyLancuch {
                nazwa: skarb.clone(),
            },
        )
        .unwrap();
        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::Auto,
            },
        )
        .unwrap();

        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &cel)]),
                tryb: Some(ui::TradingMode::Auto),
            },
        )
        .unwrap();

        let (a, ea) = st.read(|s| (s.lancuchy.aktywny.clone(), s.aktywny_ea.clone()));
        assert_eq!(a, cel, "w AUTO drabinka przestawia wspólny `aktywny`");
        assert_eq!(ea, skarb, "…i nie ma prawa tknąć składu warstwy EA");
        assert_eq!(st.workspace.load_aktywny_ea(), skarb);
    }

    /// (c) KROK PĘTLI w AUTO-EA przełącza `aktywnyEa` i TYLKO jego — razem
    /// z pamięcią szczebla, która też jest osobna dla każdego trybu.
    #[test]
    fn krok_drabinki_w_auto_ea_przelacza_tylko_wskaznik_ea() {
        let st = stan("c-krok");
        let (aktywny, dol, gora) = trzy_lancuchy(&st);
        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::AutoEa,
            },
        )
        .unwrap();
        apply(&st, &Command::SetAktywnyLancuch { nazwa: dol.clone() }).unwrap();
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &dol), (500.0, &gora)]),
                tryb: None, // brak adresu = tryb bieżący (kontrakt zera)
            },
        )
        .unwrap();
        assert_eq!(
            st.read(|s| s.aktywny_ea.clone()),
            dol,
            "na starcie stoimy na szczeblu bazowym"
        );

        // saldo przekracza próg 500 → krok pętli ma wejść wyżej
        let przelaczono = drabinka_krok(&st, 620.0);
        assert_eq!(przelaczono.as_deref(), Some(gora.as_str()));

        let (a, ea, prog_ea, prog_wsp) = st.read(|s| {
            (
                s.lancuchy.aktywny.clone(),
                s.aktywny_ea.clone(),
                s.drabinka_ea.biezacy_prog,
                s.drabinka.biezacy_prog,
            )
        });
        assert_eq!(ea, gora);
        assert_eq!(
            a, aktywny,
            "krok drabinki EA nie rusza składu pozostałych trybów"
        );
        assert_eq!(
            prog_ea, 500.0,
            "pamięć szczebla należy do drabinki, która się przesunęła"
        );
        assert_eq!(prog_wsp, -1.0, "…i nie ma prawa zapisać się w cudzej");
        assert_eq!(st.workspace.load_lancuchy().aktywny, aktywny);
        assert_eq!(st.workspace.load_aktywny_ea(), gora);
    }

    /// (c') Ten sam krok w AUTO rusza `lancuchy.aktywny` i nie dotyka EA.
    #[test]
    fn krok_drabinki_w_auto_przelacza_tylko_aktywny() {
        let st = stan("c-auto");
        let (_, dol, gora) = trzy_lancuchy(&st);
        apply(&st, &Command::SetAktywnyLancuch { nazwa: dol.clone() }).unwrap();
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &dol), (500.0, &gora)]),
                tryb: Some(ui::TradingMode::Auto),
            },
        )
        .unwrap();

        assert_eq!(drabinka_krok(&st, 900.0).as_deref(), Some(gora.as_str()));
        let (a, ea, prog_ea) = st.read(|s| {
            (
                s.lancuchy.aktywny.clone(),
                s.aktywny_ea.clone(),
                s.drabinka_ea.biezacy_prog,
            )
        });
        assert_eq!(a, gora);
        assert_eq!(ea, "", "tryb AUTO nie zakłada wskazania warstwie EA");
        assert_eq!(prog_ea, -1.0);
    }

    /// (d) STRAŻ: komenda adresowana do drabinki innego trybu = ODMOWA
    /// z wpisem w dzienniku. Cichy zapis „do właściwego pola" byłby gorszy:
    /// użytkownik ustawiałby jedno, a bot dostawał drugie.
    #[test]
    fn straz_odmawia_drabinki_spoza_trybu_i_pisze_do_dziennika() {
        let st = stan("d-straz");
        let (_, cel, _) = trzy_lancuchy(&st);
        let przed = st.read(|s| (s.drabinka.clone(), s.drabinka_ea.clone()));

        // AUTO → komenda opisująca drabinkę AUTO-EA
        let e = apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &cel)]),
                tryb: Some(ui::TradingMode::AutoEa),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("ODMOWA"),
            "komunikat ma nazwać odmowę wprost: {e}"
        );
        assert!(e.contains("AUTO-EA"), "…i wskazać, czyja to drabinka: {e}");

        let (blad, tresc) = st.read(|s| {
            let l = s
                .logs
                .iter()
                .find(|l| l.title.contains("Odmowa: drabinka spoza"))
                .expect("odmowa MUSI zostawić wpis w dzienniku");
            (l.level.clone(), l.content.clone())
        });
        assert_eq!(
            blad, "error",
            "cicha odmowa to ta sama awaria, tylko trudniejsza do znalezienia"
        );
        assert!(
            tresc.contains("SKYNET-1"),
            "wpis ma nazwać drabinkę po imieniu: {tresc}"
        );

        // AUTO-EA → komenda opisująca drabinkę wspólną
        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::AutoEa,
            },
        )
        .unwrap();
        let e = apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &cel)]),
                tryb: Some(ui::TradingMode::Auto),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("ODMOWA"), "{e}");

        assert_eq!(
            st.read(|s| (s.drabinka.clone(), s.drabinka_ea.clone())),
            przed,
            "odrzucona komenda nie ma prawa nic zmienić w ŻADNEJ z drabinek"
        );
    }

    /// (d') To samo dla wskaźnika łańcucha: `aktywnyEa` przyjmuje wyłącznie
    /// tryb, do którego należy.
    #[test]
    fn straz_odmawia_zapisu_wskaznika_ea_spoza_trybu() {
        let st = stan("d-lanc");
        let (_, inny, _) = trzy_lancuchy(&st);
        let z = st.read(|s| s.lancuchy.clone());

        let e = apply(
            &st,
            &Command::SetLancuchy {
                lancuchy: z,
                aktywny_ea: Some(inny.clone()),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("ODMOWA"), "{e}");
        assert_eq!(
            st.read(|s| s.aktywny_ea.clone()),
            "",
            "odrzucony zapis nic nie ustawia"
        );
        assert!(st.read(|s| s
            .logs
            .iter()
            .any(|l| l.level == "error" && l.title.contains("Odmowa: wskaźnik AUTO-EA"))));
    }

    /// IZOLACJA SKUTECZNOŚCI: drabinka nie-swojego trybu jest BEZCZYNNA,
    /// ale wyłącznik użytkownika przeżywa i wraca razem z trybem.
    ///
    /// To jest zamek, który działa NIEZALEŻNIE od tego, czy czytelnik pyta
    /// o tryb — a takich czytelników (pętla handlowa, lista szczebli panelu,
    /// pieczęć paczki) jest dziś kilku w trzech skrzyniach.
    #[test]
    fn drabinka_nie_swojego_trybu_jest_bezczynna_ale_wlacznik_zostaje() {
        let st = stan("skutecznosc");
        let (_, cel, _) = trzy_lancuchy(&st);
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &cel)]),
                tryb: None,
            },
        )
        .unwrap();
        assert!(
            st.read(|s| s.drabinka.enabled),
            "w swoim trybie drabinka jest skuteczna"
        );

        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::AutoEa,
            },
        )
        .unwrap();
        let d = st.read(|s| s.drabinka.clone());
        assert!(!d.enabled, "poza swoim trybem drabinka NIE MOŻE nic wybrać");
        assert_eq!(
            d.wlacznik,
            Some(true),
            "…ale wyłącznik użytkownika zostaje zapamiętany"
        );
        assert!(
            d.wybierz(10_000.0).is_none(),
            "bezczynna drabinka nie wskazuje szczebla"
        );
        assert!(
            !st.read(|s| s.drabinka_biezaca().enabled),
            "drabinka EA nie była układana"
        );

        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::Auto,
            },
        )
        .unwrap();
        assert!(
            st.read(|s| s.drabinka.enabled),
            "powrót trybu przywraca drabinkę bez pytania"
        );
    }

    /// (e) KONTRAKT ZERA: pamięć sprzed EA-2c nie zna ani `drabinkaEa`, ani
    /// `wlacznik`. Ma wczytać się bez ostrzeżenia i dać DOKŁADNIE to, co dawała.
    #[test]
    fn stara_pamiec_bez_drabinki_ea_dziala_jak_dotad() {
        let st = stan("e-zero");
        let (_, cel, _) = trzy_lancuchy(&st);
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &cel)]),
                tryb: None,
            },
        )
        .unwrap();
        st.save_backup().unwrap();

        // …i cofamy plik do formatu sprzed tej wersji
        let sciezka = st.workspace.backup_latest();
        let mut v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&sciezka).unwrap()).unwrap();
        {
            let o = v.as_object_mut().unwrap();
            o.remove("drabinkaEa")
                .expect("nowy zapis MUSI nieść drabinkę EA");
            o.get_mut("drabinka")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove("wlacznik");
        }
        std::fs::write(&sciezka, serde_json::to_vec_pretty(&v).unwrap()).unwrap();

        let cfg = crate::ServerConfig {
            workspace: st.workspace.root.clone(),
            ..Default::default()
        };
        let st2 = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
        let (d, dea) = st2.read(|s| (s.drabinka.clone(), s.drabinka_ea.clone()));
        assert!(
            d.enabled,
            "stara drabinka wraca WŁĄCZONA — tak jak ją zostawiono"
        );
        assert_eq!(
            d.wlacznik,
            Some(true),
            "intencję odtwarzamy z `enabled`, bo innej nie było"
        );
        assert_eq!(d.szczeble.len(), 1);
        assert!(
            dea.szczeble.is_empty(),
            "AUTO-EA nie dostaje planu, którego nikt nie ułożył"
        );
        assert!(!dea.enabled);
    }

    /// (f) PODZIAŁ PRZEŻYWA RESTART — razem z trybem, wyłącznikami obu
    /// drabinek i pamięcią szczebla.
    #[test]
    fn podzial_przezywa_restart() {
        let st = stan("f-restart");
        let (_, wsp, ea) = trzy_lancuchy(&st);
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &wsp)]),
                tryb: None,
            },
        )
        .unwrap();
        apply(
            &st,
            &Command::SetMode {
                mode: ui::TradingMode::AutoEa,
            },
        )
        .unwrap();
        apply(
            &st,
            &Command::SetDrabinka {
                drabinka: drabinka(&[(0.0, &ea)]),
                tryb: Some(ui::TradingMode::AutoEa),
            },
        )
        .unwrap();
        st.save_backup().unwrap();

        let cfg = crate::ServerConfig {
            workspace: st.workspace.root.clone(),
            ..Default::default()
        };
        let st2 = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
        let (tryb, d, dea) = st2.read(|s| (s.mode, s.drabinka.clone(), s.drabinka_ea.clone()));
        assert_eq!(tryb, ui::TradingMode::AutoEa);
        assert!(dea.enabled, "drabinka bieżącego trybu wstaje skuteczna");
        assert_eq!(dea.szczeble[0].lancuch, ea);
        assert!(!d.enabled, "drabinka drugiego trybu wstaje BEZCZYNNA");
        assert_eq!(d.wlacznik, Some(true), "…z zapamiętanym wyłącznikiem");
        assert_eq!(d.szczeble[0].lancuch, wsp);
        assert_eq!(
            st2.read(|s| s.aktywny_ea.clone()),
            ea,
            "skład warstwy EA przeżył restart"
        );
    }
}
