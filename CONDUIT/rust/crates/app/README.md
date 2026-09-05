# `conduit-app` — binarka `conduit.exe`

Jeden proces: serwer lokalny + okno natywne Windows. Okno i przeglądarka są
**równorzędnymi klientami tego samego stanu** i mogą działać jednocześnie.

```
┌─────────────────────┐      ┌──────────────────────────┐
│  Okno (Tauri v2)    │      │  Przeglądarka            │
│  WebView2 · 4K/DPI  │      │  http://127.0.0.1:8787   │
└──────────┬──────────┘      └───────────┬──────────────┘
           └──────── WebSocket /ws ──────┘
                        │
              conduit.exe (axum + stan)
```

---

## Uruchomienie

```bat
conduit.exe                     serwer + okno natywne
conduit.exe --headless          sam serwer (VPS, usługa Windows)
conduit.exe --open              serwer + strona w domyślnej przeglądarce
conduit.exe --demo              syntetyczne kwotowania (bez MT5)
conduit.exe --help              pełna lista opcji
```

| Opcja | Znaczenie | Domyślnie |
|---|---|---|
| `--host <adres>` | adres nasłuchu | `127.0.0.1` |
| `--port <numer>` | port | `8787` |
| `--data-dir <kat>` | katalog konfiguracji i pamięci stanu | katalog obok `conduit.exe` |
| `--web-dir <kat>` | interfejs z dysku zamiast wbudowanego | — |
| `--balance <kwota>` | saldo startowe, gdy nie ma zapisanego stanu | `2000` |
| `--headless` | bez okna natywnego | wyłączone |
| `--open` | otwórz przeglądarkę po starcie | wyłączone |
| `--demo` | syntetyczne kwotowania | wyłączone |

Zatrzymanie: **Ctrl+C** (headless) albo zamknięcie okna. W obu przypadkach
program robi ostatni zapis do `backup_memory/` — bez tego restart gubiłby
do 15 sekund pracy (tyle trwa okno zapisu cyklicznego).

---

## Tryb `--headless`

Startuje wyłącznie serwer: HTTP, REST i WebSocket. Interfejs jest dostępny
pod `http://<host>:<port>` z dowolnej przeglądarki — także zdalnie, jeśli
podasz `--host 0.0.0.0`.

> **Uwaga.** Serwer NIE MA uwierzytelniania. Domyślne `127.0.0.1` jest
> świadome: to narzędzie dla jednego użytkownika na jednej maszynie.
> Wystawienie go na `0.0.0.0` bez tunelu SSH albo VPN oznacza, że każdy
> w tej sieci może wysyłać zlecenia na Twoje konto.

Tryb headless nie wymaga WebView2 ani pulpitu, więc nadaje się na serwer
bez sesji graficznej. Można też zbudować binarkę zupełnie bez Tauri:

```bat
cargo build --release -p conduit-app --no-default-features
```

Taka wersja jest wielokrotnie szybsza w budowie i zachowuje się jak
`--headless` niezależnie od przełączników.

---

## Pliki czytane z katalogu konfiguracji

Domyślnie to katalog, w którym leży `conduit.exe` (zmiana: `--data-dir`).
Świadomie **nie** `%APPDATA%` — cały zestaw ma być przenośny: skopiuj folder
na VPS i działa.

```
conduit.exe
settings.json          konfiguracja panelu + tryb + lot + wybrany preset
smtp.json              poczta i powiadomienia
channels.json          przypisania kanałów Telegrama
window.json            zapamiętana pozycja i rozmiar okna
presets/*.json         presety (jeden plik = jeden preset)
backtests/*.json       wyniki backtestów (czyta je `GET /api/backtests`)
backup_memory/
  latest.json          ostatni zapis — z niego wznawiamy po restarcie
  2026-07-27_143012-… kopie rotacyjne (20 ostatnich)
```

Brak pliku **nie jest błędem** — program startuje na wartościach domyślnych
i przy starcie wypisuje, co znalazł:

```
  settings.json: jest · smtp.json: BRAK · channels.json: jest · presety: 12 · backup: jest
```

Uszkodzony plik jest pomijany z ostrzeżeniem w logu. Jeden zepsuty preset
nie może uniemożliwić uruchomienia bota.

### `backup_memory/` — wznawianie po restarcie

Zapis co 15 sekund oraz przy zamknięciu. Zawiera koszyki, statystyki, stan
strażnika ryzyka, historię i ostatnie 200 wiadomości (dzięki nim wiązanie
„odpowiedź → koszyk" przeżywa restart).

Zapis jest **atomowy** (`.tmp` → `rename`), więc zanik zasilania nie zostawi
obciętego JSON-a. Gdyby mimo to `latest.json` był nieczytelny, program
schodzi do najnowszej sprawnej kopii rotacyjnej — po to one są.

Pozycje i zlecenia zapisujemy jako **ostatni znany stan do rekoncyliacji**.
Źródłem prawdy po starcie zawsze pozostaje broker; gdyby było odwrotnie,
restart po ręcznym zamknięciu pozycji w terminalu wskrzeszałby duchy.

Format ma wersję (`version: 1`). Plik w innej wersji jest **odrzucany**,
a nie wczytywany „na chybił trafił".

---

## Okno natywne

- **Tauri v2 + WebView2.** Okno ładuje ten sam adres co przeglądarka
  (`http://127.0.0.1:8787/?shell=native`), więc nie istnieje druga
  implementacja interfejsu, która mogłaby się rozjechać.
- **4K / DPI** obsługuje WebView2; Tauri ustawia świadomość DPI w manifeście.
  Interfejs jest zbudowany na jednostkach względnych, więc ostrość jest
  darmowa — nic nie trzeba przeliczać.
- **Geometria okna** (pozycja, rozmiar, maksymalizacja) jest zapisywana
  w `window.json` przy każdym ruchu i przy zamknięciu, w pikselach
  **logicznych** — przeniesienie okna między monitorem 4K a zwykłym nie
  zmienia jego rozmiaru na ekranie. Minimalny rozmiar: 1024×640.
- **Ikona w zasobniku** z menu: „Pokaż okno", „Otwórz w przeglądarce",
  „Zakończ". Ikona jest rysowana w kodzie, żeby binarka pozostała jednym
  plikiem.
- **Przycisk „Otwórz w przeglądarce"** jest w szynie nawigacji (na dole,
  nad kartą użytkownika) i pojawia się tylko w oknie natywnym. Woła
  `POST /api/shell/open-browser`, bo w WebView2 `window.open()` otworzyłby
  kolejny webview zamiast przeglądarki systemowej. Endpoint nie przyjmuje
  żadnego parametru — otwiera wyłącznie własny adres serwera.

Wymaga **WebView2 Runtime** (obecny w Windows 11 i aktualizowanym Windows 10).
Gdy go brakuje, `conduit.exe` zgłasza to wprost przy starcie okna; serwer
działa dalej, więc interfejs pozostaje dostępny w przeglądarce.

---

## Budowanie

Interfejs jest wkompilowany w binarkę (`rust-embed`), więc trzeba go najpierw
zbudować i skopiować:

```bat
npm run build
cargo build --release -p conduit-server -p conduit-app
```

Wynik: `rust\target\release\conduit.exe` — jeden plik do skopiowania.

`npm run build` uruchamia także `narzedzia/panel_do_exe.mjs`, który kopiuje
interfejs do katalogów osadzanych w binarkach Rust. Bez tego kroku binarka
serwuje stronę z instrukcją zamiast pustego ekranu, a API działa normalnie.

Przy pracy nad UI nie ma sensu kompilować Rusta po każdej zmianie CSS:

```bat
conduit.exe --web-dir ..\dist        interfejs z dysku
npm run dev                          Vite na :5180, backend na :8787 (CORS wpuszcza localhost)
```

---

## Testy

```bat
cargo test -p conduit-server
```

Pokrywają: serializację protokołu WS, koalescencję delt do ~10 Hz, zapis
i odczyt `backup_memory` (w tym uszkodzony `latest.json` i odrzucenie starej
wersji formatu) oraz — w `tests/ws_e2e.rs`, na prawdziwym gnieździe —
dwóch jednoczesnych klientów widzących tę samą zmianę.
