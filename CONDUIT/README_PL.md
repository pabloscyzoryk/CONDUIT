# CONDUIT — kod źródłowy

CONDUIT jest terminalem kopiującym sygnały z Telegrama do MetaTrader 5. Łączy
odbiór i wersjonowanie wiadomości, parser wielu formatów, wspólny silnik
zarządzania koszykiem, egzekucję u brokera, panel React oraz tester tickowy.

Repozytorium jest publiczną, oczyszczoną kopią **samego kodu źródłowego**. Nie
zawiera sesji Telegrama, `api_id`/`api_hash`, haseł, danych rachunku MT5,
identyfikatorów prywatnych kanałów i tematów, logów, ticków, eksportów czatów,
wyników sweepów ani binarek.

## GOD-X7 a silnik CONDUIT

`config/presets/` zawiera **109 publicznych presetów** zebranych w toku rozwoju.
`GOD-X7.json` jest aktualnie rekomendowanym punktem odniesienia dla formatu
Synergy, ale nie jest osobnym botem i nie wyczerpuje możliwości projektu. Rdzeń CONDUIT ma
**520 pól ustawień rdzenia**: wejścia i siatki, sizing, limity ekspozycji,
TP/SL, partiale, break-even/risk-free, runnery, trailing, re-entry, filtry
reżimu i sesji, koszty brokera, zachowanie zleceń pending, dziennik i wiele
kontraktów zgodności live/backtest. Oś wpływa na wynik tylko wtedy, gdy jest
włączona oraz nie jest przesłonięta przez nadrzędną regułę lub ustawienie
łańcucha.

Wbudowane formaty obejmują obecnie `ATFX`, `Synergy`, `ZEN`, `PULSEX`, `NOVA`,
`TWP`, `DANGER`, `CLUB1` i `STORM`. Dla każdego źródła można przypisać osobny
preset albo pozostawić je w trybie nasłuchu bez handlu. Dostępne tryby to m.in.
`MANUAL`, `AUTO`, `AUTO-EA` i `AI`; ich dokładne znaczenie opisuje panel.

## Architektura

```text
Telegram MTProto ──> adapter historii/live ──> parser formatu
                                               │
Panel React <──WebSocket/REST── serwer Axum <──┤
                                               v
                                      conduit-core Engine
                                      /                 \
                              SimBroker (backtest)   Broker trait (live)
                                                        │
                                             Rust MT5 bridge/TCP
                                                        │
                                             mt5_sidecar.py
                                                        │
                                            MetaTrader5 Python API
                                                        │
                                              terminal MetaTrader 5
```

- `src/` — panel TypeScript/React, schemat osi i obsługa transportu.
- `rust/crates/core/` — parser, stan koszyków i wspólna logika decyzji.
- `rust/crates/backtest/` — deterministyczny broker symulowany, ticki i metryki.
- `rust/crates/telegram/` — logowanie MTProto, sesja, wiadomości i edycje.
- `rust/crates/mt5/` — most Rust ↔ Python ↔ MetaTrader 5.
- `rust/crates/server/` — REST/WebSocket, stan panelu, konfiguracja i dziennik.
- `rust/crates/app/` — docelowy program `conduit` i okno Tauri/WebView2.
- pozostałe crate’y — monitor sweepów, eksport, kronika, wykresy i eksperymentalna
  warstwa AI; są częścią workspace i kompilują się z tego samego drzewa.
- `mql5/` — źródło eksperta testera strategii; pliki `.ex5` nie są dołączone.

Backtest i live używają tego samego `conduit-core::Engine`. Różni je
implementacja interfejsu brokera: tester dostaje `SimBroker`, a live most MT5.
To ogranicza różnice logiki, ale nie usuwa fizycznego spreadu, slippage,
opóźnienia sieci, odmowy zleceń, requote ani zmian specyfikacji symbolu.

## Wymagania

- Windows 10/11 x64 (docelowy terminal MT5 i okno aplikacji).
- Rust **1.85+** z toolchainem MSVC (`rustup`, Visual Studio Build Tools z
  „Desktop development with C++” i Windows SDK).
- Node.js zgodny z Vite 7: co najmniej 20.19 albo 22.12; npm z lockfile.
- Microsoft Edge WebView2 Runtime dla okna Tauri.
- MetaTrader 5 x64.
- Python x64 3.8+ widoczny jako `python` oraz pakiet `MetaTrader5`.
- Własne dane aplikacji Telegram: `api_id` i `api_hash` z
  <https://my.telegram.org>.
- MetaEditor 5 tylko jeśli chcesz kompilować źródła z `mql5/`.

## Budowanie od zera

W PowerShell, w katalogu repozytorium:

```powershell
npm ci
npm run build
npm run build:kronika
npm run build:wiz

Set-Location rust
cargo build --release -p conduit-app --bin conduit
```

Kolejność jest ważna. Panel jest osadzany w binarce przez `rust-embed`, więc
frontend musi zostać zbudowany **przed** Rustem. Wynik główny znajduje się w
`rust/target/release/conduit.exe`.

Wariant bez natywnego okna, przydatny do diagnostyki serwera:

```powershell
Set-Location rust
cargo build --release -p conduit-app --bin conduit --no-default-features
```

Tester CLI:

```powershell
Set-Location rust
cargo build --release -p conduit-backtest --bin btp
./target/release/btp.exe --help
```

Kompilacja eksperta MQL5 odbywa się w MetaEditorze: otwórz
`mql5/CONDUIT_XT.mq5`, skompiluj i przeczytaj wszystkie komunikaty kompilatora.
Ekspert testera nie jest wymagany do połączenia programu live przez sidecar.

Przygotowanie Pythona:

```powershell
py -3 -m venv .venv
./.venv/Scripts/python.exe -m pip install --upgrade pip
./.venv/Scripts/python.exe -m pip install MetaTrader5
```

W ustawieniach aplikacji wskaż ten interpreter albo dodaj go do `PATH`.
Do katalogu wdrożenia obok `conduit.exe` skopiuj
`rust/crates/mt5/sidecar/mt5_sidecar.py`.

## Pierwsze uruchomienie z własnymi danymi

1. Utwórz pusty katalog wdrożenia i skopiuj do niego `conduit.exe` oraz
   `mt5_sidecar.py`.
2. Skopiuj pliki z `config/examples/` obok programu, usuwając `.example` z
   nazwy. Skopiuj cały katalog `config/presets/` jako katalog `presets/` obok
   programu. GOD-X7 jest rekomendowany, ale panel może ładować wszystkie pliki
   JSON obecne w tym katalogu.
3. Uruchom MT5, zaloguj **własny rachunek demo**, włącz Algo Trading i upewnij
   się, że właściwy symbol złota jest widoczny w Market Watch.
4. Uruchom CONDUIT. Podaj własne `api_id`/`api_hash`, zeskanuj kod QR w
   Telegramie i zakończ logowanie. Sekret i sesja powstaną lokalnie — nigdy
   ich nie commituj.
5. W zakładce kanałów odśwież dialogi, wybierz kanał lub temat, włącz nasłuch
   i przypisz **prawidłowy format wiadomości**. ID jest pobierane z Twojego
   konta; w repozytorium nie ma gotowych prywatnych powiązań.
6. W łańcuchu przypisz wybrany preset do danego formatu. Neutralny szablon
   startuje bez automatycznego handlu; GOD-X7 jest wskazanym presetem
   referencyjnym, a pozostałe formaty można pozostawić listen-only.
7. Pozostań w `MANUAL`, sprawdź parsowanie i zlecenia na demo, następnie testuj
   `AUTO` na demo. Dopiero po własnej walidacji rozważ rachunek rzeczywisty.

## Presety i własny workflow

Preset jest zwykłym JSON-em z wartościami osi. Najbezpieczniejszy proces:

1. Skopiuj istniejący preset i nadaj nową nazwę; nie edytuj wzorca w miejscu.
2. Zmień jedną rodzinę osi naraz i zapisuj hash/wersję konfiguracji.
3. Przetestuj identyczny korpus ticków i wiadomości, koszty oraz zegar.
4. Sprawdź pełne okno, dni/tygodnie osobno, różne kapitały i limity lota.
5. Wykonaj testy odporności i dopiero potem demo live.
6. Przed produkcją sprawdź, czy panel/łańcuch/rachunek nie nadpisują pól
   presetu. Pliki runtime i preset muszą mieć oczekiwaną wersję i hash.

Żaden z dołączonych presetów nie jest obietnicą optimum dla innego brokera,
miesiąca, formatu lub ryzyka. GOD-X7 jest rekomendacją bieżącej wersji, nie
gwarancją wyniku. Ustawienia tworzą bardzo dużą przestrzeń interakcji; wynik
jednego sweepu może być nadstrojony.

## Bezpieczeństwo: DEMO → REAL

Szablon startuje w `MANUAL` i ma `mt5_allow_real_account=false`.

- Nigdy nie włączaj REAL tylko dlatego, że projekt się kompiluje.
- Najpierw potwierdź identyfikator konta, tryb DEMO/REAL, serwer, symbol,
  `digits`, `volume_min/step/max`, `stops_level`, fill policy, spread i swap.
- Nie uruchamiaj dwóch instancji sterujących tym samym magic/symbolem.
- Ustaw limity ryzyka adekwatne do rachunku; wartości 0 często oznaczają brak
  danego limitu, a nie maksymalne bezpieczeństwo.
- Zrób kontrolowane próby: nowy sygnał, edycja SL/TP, cancel, TP1/TP2/TP3,
  partial, restart, utrata sidecara i ręczne przełączenie konta.
- Dopiero po udokumentowanej zgodności demo ustaw zgodę na REAL świadomie.

## Testy

Po zbudowaniu frontendów:

```powershell
npm run typecheck

# Testy kontraktów źródeł MQL/UI (wymagają wcześniej zbudowanego przykładu Rust):
Set-Location rust
cargo build -p conduit-server --example ui_alias_probe
Set-Location ..
npm run test:node

Set-Location rust
cargo test --workspace --lib --offline
cargo test -p conduit-mt5 --offline
```

Testy wymagające prywatnych korpusów, ticków albo zewnętrznego terminala nie są
częścią publicznego repo i powinny jawnie zgłosić brak danych albo zostać
pominięte. Testy Pythona sidecara:

```powershell
Set-Location rust/crates/mt5/sidecar
python -m unittest discover -p "test_*.py"
```

Flaga Cargo `--offline` działa tylko, gdy lokalny cache zawiera już wszystkie
crates z `Cargo.lock`. Repozytorium nie vendoruje zależności Rust ani npm;
na świeżej maszynie uruchom pierwszy `cargo fetch`/`cargo test` i `npm ci`
z dostępem do sieci, a dopiero później używaj trybu offline.

## Rozwiązywanie problemów

- **Stary panel w nowej binarce:** usuń `dist`/wygenerowane `web`, wykonaj trzy
  buildy npm, a dopiero potem `cargo build`.
- **`rust-embed` nie widzi `web/`:** nie pomijaj kroku frontendowego.
- **MT5 disconnected:** uruchom terminal x64, sprawdź interpreter Pythona i
  `python -c "import MetaTrader5"`, ścieżkę terminala oraz jeden aktywny
  sidecar.
- **Zły `XAUUSD`/`XAUUSD.s`:** pozostaw symbol w auto-detekcji albo wybierz
  dokładną nazwę z bieżącego serwera; po zmianie konta sprawdź ponownie.
- **Telegram nie pokazuje kanałów:** sprawdź własne API credentials, zakończ
  stare sesje tylko jeśli rozumiesz skutek, zaloguj ponownie i odśwież dialogi.
- **Sygnały są widoczne, ale bot nie handluje:** sprawdź nasłuch, topic,
  przypisany format, preset w aktywnym łańcuchu, tryb, status halt i walidację
  sygnału. Listen-only jest poprawnym stanem.
- **Backtest różni się od live:** najpierw porównaj zegar, korpus edycji,
  symbol, spread, koszty, pending fill, stops level, opóźnienie i ustawienia
  rachunku/łańcucha; slippage i broker rejection nie są przyszłością znaną
  testerowi.

## Dane, które muszą pozostać poza GitHubem

Nie commituj: `secrets.json`, plików `*.session`, `.env`, ustawień z loginem,
`channels.json` po mapowaniu, `lancuchy.json` z prywatnymi bindingami, logów,
kroniki, eksportów Telegrama, historii MT5, ticków, raportów, dumpów, modeli
wytrenowanych na prywatnych danych ani binarek. `.gitignore` obejmuje typowe
przypadki, ale przed publikacją zawsze wykonaj własny skan historii Git.

## Licencja i ryzyko

Projekt źródłowy nie zawierał jednoznacznej licencji publicznej, dlatego ta
kopia również nie dodaje wymyślonej licencji. Domyślne prawa autorskie nadal
obowiązują. Oprogramowanie nie gwarantuje zysku ani zgodności z regulacjami
Twojej jurysdykcji. Handel lewarowany może szybko wyzerować rachunek.
