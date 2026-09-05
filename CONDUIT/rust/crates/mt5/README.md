# conduit-mt5 — most do MetaTrader 5

Implementacja cechy `Broker` z `conduit-core` na żywym koncie MT5.
Ten sam interfejs, którym posługuje się symulator z backtestu — dzięki temu
silnik nie wie, czy handluje naprawdę, czy liczy historię.

```text
Engine ──(Broker)──▶ Mt5Bridge ──▶ Transport ──TCP 127.0.0.1──▶ mt5_sidecar.py ──▶ terminal
```

## Dlaczego sidecar w Pythonie

MetaTrader 5 nie ma API dla Rusta i nie zapowiada się, żeby miał. Realne opcje:

| droga | koszt | stan |
|---|---|---|
| pakiet `MetaTrader5` (Python) przez lokalny socket | jeden proces w tle, ułamek ms na wywołanie | **wdrożone** |
| Expert Advisor w MQL5 przez nazwany potok | brak IPC, ticki ze zdarzeń zamiast odpytywania | szkic: `mql5/ConduitBridge.mq5` |
| wstrzykiwanie do procesu terminala | kruche, nieutrzymywalne | odrzucone |

Sidecar mówi tym samym protokołem, co szkic EA — podmiana transportu nie
wymaga zmian w Ruście.

## Wymagania

* **Windows** i **64-bitowy Python 3.8+** (pakiet `MetaTrader5` jest tylko taki):
  ```
  pip install MetaTrader5
  ```
* Zainstalowany i **uruchomiony** terminal MetaTrader 5.
* W terminalu: *Narzędzia → Opcje → Doradcy → Zezwalaj na handel algorytmiczny*.
* Symbol (domyślnie `XAUUSD`) widoczny w *Podglądzie rynku*.

## Konfiguracja

```rust
use conduit_mt5::{Mt5Bridge, SidecarConfig, default_sidecar_path};

let cfg = SidecarConfig {
    python: "python".into(),              // albo pełna ścieżka do python.exe
    script: default_sidecar_path(),       // crates/mt5/sidecar/mt5_sidecar.py
    terminal_path: Some(r"C:\Program Files\MetaTrader 5\terminal64.exe".into()),
    login: Some(123456789),               // None = konto już zalogowane w terminalu
    password: Some("…".into()),           // trafia zmienną środowiskową, nie argumentem
    server: Some("Vantage-Live".into()),
    symbol: "XAUUSD".into(),
    magic: 770_077,
    ..Default::default()
};
let mut b = Mt5Bridge::connect(cfg)?;     // podnosi sidecar i odtwarza stan z konta

loop {
    for q in b.poll() {                   // nowe kwotowania
        engine.on_tick(&mut b, &q);
    }
}
```

Najprościej jest **nie podawać** `login`/`password`/`server` i po prostu zalogować
się w terminalu ręcznie — sidecar podłączy się do bieżącej sesji.

### Ważne parametry

| pole | znaczenie |
|---|---|
| `magic` | znacznik naszych zleceń. Pozycje z innym `magic` są **niewidoczne** dla bota |
| `symbol` | jeden instrument na jeden most |
| `deviation_points` | dopuszczalny poślizg zlecenia rynkowego |
| `tick_interval_ms` | co ile sidecar odpytuje o kwotowanie (domyślnie 50 ms) |
| `autostart` | `false` = sidecar uruchamiasz sam (do diagnostyki) |

## Protokół

Jedna linia = jeden dokument JSON zakończony `\n`, UTF-8.
**Serwerem jest Rust**, klientem sidecar — port wybiera system (`bind` na 0),
więc nie ma wyścigu o port ani zgadywania, ile wstaje terminal.

```jsonc
// żądanie
{"id":7,"cmd":"open_market","args":{"side":"buy","volume":0.01, ...}}
// odpowiedź
{"id":7,"ok":true,"result":{"retcode":10009,"order":123,"deal":456,"position":123}}
{"id":7,"ok":false,"error":{"code":10016,"msg":"Invalid stops"}}
// zdarzenia (bez `id`)
{"ev":"tick","ts":1700000000000,"bid":4000.00,"ask":4000.24}
{"ev":"closed","position":42,"profit":9.8,"reason":5, ...}
```

Sidecar można uruchomić ręcznie:
```
python crates/mt5/sidecar/mt5_sidecar.py --port 51234 --symbol XAUUSD --magic 770077
```

## Decyzje projektowe, które mają znaczenie

### Parametry instrumentu pochodzą z serwera
`digits`, `point`, `stops_level`, krok wolumenu i tryb wypełnienia są
odczytywane przez `symbol_info`. Nic nie jest zaszyte w kodzie: `stops_level=20`
prawdziwe u jednego brokera cicho psuje wyniki u każdego innego.

### Odświeżenie stanu SCALA, nie nadpisuje
`Position` niesie pola, których broker nie zna: wirtualny SL, szczyt zysku,
`frozen`, `is_runner`. Podmiana wektora pozycji na świeży odczyt z MT5
kasowałaby je co pół sekundy — trailing nigdy by nie ruszył.

### Cudze pozycje są niewidoczne
Do pamięci mostu trafia tylko to, co ma nasz `magic` **i** nasz symbol.
Silnik ma `close_everything()`; bez tego filtra zamknąłby ręczne pozycje
użytkownika razem ze swoimi.

### Semantyka zestrojona z symulatorem
* SL/TP niewykonalny jest odrzucany **przed** wysłaniem (ta sama reguła, co
  `sl_is_valid` w rdzeniu),
* limit ustawiony za rynkiem jest zamieniany na zlecenie rynkowe — tak samo
  jak w `SimBroker`; inaczej backtest i konto rozjeżdżałyby się dokładnie
  na najbardziej dynamicznych sygnałach.

### Ponawianie tylko tam, gdzie jest bezpieczne
Requote, „cena się zmieniła", „cena niedostępna", chwilowy zator → ponawiamy
z odświeżoną ceną. „Invalid stops", „brak marginu", „rynek zamknięty" → od razu
błąd. **Timeout zlecenia handlowego NIE jest ponawiany** — nie wiadomo, czy
zlecenie doszło, a ponowienie mogłoby otworzyć drugą pozycję; zamiast tego
wymuszana jest rekoncyliacja (licznik `unknown_sends`).

### Rekoncyliacja po restarcie
Numer koszyka, poziom siatki i flaga touchera są kodowane w komentarzu zlecenia
(`CD12.3t`, moduł `comment`) — komentarz MT5 ma 31 znaków, więc część maszynowa
idzie na początek i przeżywa obcięcie przez brokera. `Mt5Bridge::reconcile()`
odtwarza z tego stan i raportuje, ile pozycji było naszych, ile bezpańskich,
a ile cudzych.

## Testy

```
cargo test -p conduit-mt5      # 36 testów, bez terminala i bez sieci
```

Pokrywają: parsowanie protokołu, mapowanie retcodes MT5 na `BrokerError`,
ponawialność, kodowanie/odczyt komentarza (z obcięciem i sufiksem brokera),
przeliczanie `stops_level` z punktów oraz **pełną pętlę transportu z atrapą
sidecara** (powitanie, żądanie/odpowiedź, odmowa brokera, strumień ticków
i zamknięć, opróżnianie buforów).

## CZEGO NIE UDAŁO SIĘ ZWERYFIKOWAĆ

Nie miałem dostępu do terminala MT5 ani do konta brokerskiego. **Niesprawdzone
na żywym systemie jest wszystko, co dotyka pakietu `MetaTrader5`:**

* skrypt `mt5_sidecar.py` przeszedł tylko kontrolę składni (`ast.parse`) —
  **nie został uruchomiony ani razu**, bo pakiet `MetaTrader5` nie da się
  zaimportować bez terminala;
* nazwy i typy pól zwracanych przez `positions_get`, `orders_get`,
  `history_deals_get`, `symbol_info`, `account_info` przyjęto z dokumentacji
  MetaQuotes — nie zostały potwierdzone realnym odczytem;
* dobór trybu wypełnienia (maska `filling_mode` → `ORDER_FILLING_*`)
  i obsługa odmowy `10030` nie zostały sprawdzone u żadnego brokera;
* wyznaczanie tiketu POZYCJI z tiketu deala (`position_id`) — logika napisana
  pod konto **hedging**; na koncie **netting** dokładanie do istniejącej
  pozycji zachowa się inaczej i wymaga osobnego sprawdzenia;
* okno przeglądania historii dealów jest szerokie (±26 h), bo historia MT5
  jest znakowana czasem SERWERA, a `time.time()` daje czas lokalny UTC;
  przed dublami broni zbiór widzianych tiketów, ale **realnego przesunięcia
  stref nie zmierzyłem**;
* rzeczywiste opóźnienie i liczba gubionych ticków przy odpytywaniu co 50 ms
  są nieznane — to jest główny argument za przejściem na wariant MQL5;
* `ConduitBridge.mq5` **nie był kompilowany** w MetaEditorze (brak MT5).
  To jest jawnie oznaczony szkic; brakuje w nim pełnego parsera JSON,
  `close_partial` i `modify_pending`, a strona Rusta nie ma jeszcze wariantu
  `Transport` na nazwane potoki.

Zweryfikowane bez konta jest natomiast to, co wymieniono w sekcji „Testy" —
w tym pełna pętla transportu, nadzór nad procesem i mapowanie błędów.

Jeden realny błąd został znaleziony i naprawiony właśnie przez ten test:
na Windows gniazdo zwrócone przez `accept()` dziedziczy tryb nieblokujący po
nasłuchu, przez co nadzór uznawał żywy sidecar za martwy i restartował go
w nieskończoność.
