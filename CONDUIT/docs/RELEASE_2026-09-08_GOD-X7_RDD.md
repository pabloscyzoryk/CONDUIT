# CONDUIT — GOD-X7, RDD i diagnostyka wykonania

GOD-X7 pozostaje głównym presetem. Aktualizacja dodaje Real Drawdown i poprawia
rozróżnianie zwykłego oczekiwania na potwierdzenie od rzeczywistego problemu
wykonania. Nie zmienia ustawień strategii ani reguł odczytu sygnałów.

## Real Drawdown

Prostokąt Drawdown zawiera mały dopisek `RDD: x,y%`. Dymek po najechaniu myszą
lub ustawieniu fokusu pokazuje definicję, kwotę w walucie rachunku i procent.
RDD to strata od początkowego equity dnia do najniższego zaobserwowanego equity,
bez liczenia wcześniejszego wzrostu jako dodatkowej straty. Dla
`200 → 250 → 180 → 230` wynosi 20 i 10%.

RDD korzysta z potwierdzonych odczytów rachunku i doby brokera. Nie resetuje się
przy odbiciu equity ani wznowieniu po blokadzie ryzyka. Przy braku początku dnia
lub danych w starszym backupie pozostaje niedostępne. Znane minimum tego samego
dnia zachowuje się przy odtworzeniu stanu; zmiana rachunku nie przenosi jego RDD.
Procent wymaga dodatniego equity początkowego. Szczegóły opisuje dokument
[Real Drawdown](../../docs/real-drawdown.md).

Backtest zapisuje dzienne minimum equity, RDD i podstawę obserwacji. Maksymalny
drawdown obejmuje również wyceny pomiędzy punktami zapisanego wykresu. Może być
wyższy od starej wartości liczonej z przerzedzonej krzywej mimo identycznych
transakcji i zysku. Starsze wyniki nie otrzymują RDD przez zgadywanie minimum.

## Potwierdzenia operacji i kwotowania

Krótkie oczekiwanie na rozliczenie zamknięcia nie generuje już czerwonego alarmu.
Po pięciu sekundach trwającego oczekiwania pojawia się jedno ostrzeżenie.
Sprzeczne lub wymagające sprawdzenia potwierdzenie nadal natychmiast zgłasza
błąd. Bramka nowych wejść zachowuje dotychczasowe zasady.

Dziennik odróżnia lokalne odroczenie, niepewne wykonanie i zdalną odmowę.
Diagnozę uzupełniają numer operacji, próby wysyłki, retcode i wybrane pola
potwierdzenia. Dane logowania, tożsamość rachunku i dowolna treść odpowiedzi
nie należą do tego dodatkowego zapisu.

Sam brak nowych cen nie jest nazywany utratą połączenia. Kontrola stanu terminala
rozróżnia potwierdzone połączenie, potwierdzony brak połączenia i brak wiedzy.
Kontrolne odtworzenia po samej ciszy występują coraz rzadziej: po 12 minutach,
potem w odstępach 24, 48 i maksymalnie 60 minut. Powrót tej samej starej kwoty
po odtworzeniu nie zeruje licznika ciszy. Świeża kwota, w tym po cofnięciu
zegara brokera, kończy epizod. Wykryty problem transportu nadal prowadzi do
ścieżki odzyskania połączenia.

## Uzupełnienie: nagranie live i rearm

Nagranie live obejmuje wejścia i stan silnika oraz odpowiedzi brokera w
rzeczywistej kolejności. Eksport `alllogs` domyślnie dołącza zachowane sesje
z kontrolą integralności. Osobny tryb offline sprawdza decyzje bez połączenia
z Telegramem lub MT5. Zakres, ograniczenia i prywatność opisuje
[odtwarzanie sesji live](LIVE_REPLAY.md).

Po niejednoznacznym potwierdzeniu otwarcia rearm mógł wcześniej przyjąć później
wykrytą pozycję bez naliczenia próby i jej cooldownu. Teraz potrzebne jest
ścisłe powiązanie z wysłanym zleceniem i rachunkiem. Potwierdzona siatka
zostaje naliczona raz, a cooldown zaczyna się od pierwotnej próby wysłania.
Częściowy ACK nie nalicza siatki ponownie; zwykła odmowa jej nie nalicza.

Nierozstrzygnięta próba blokuje nową ekspozycję, zachowując możliwość zamykania
i ochrony pozycji. Gdy dowód wykonania jest niewystarczający, program wymaga
sprawdzenia zamiast zgadywać wynik. Stan uzgodnienia przetrwa restart na tym
samym rachunku. Ustawienia GOD-X7 pozostają takie same.

## Historia brokera w allLogs

Nowa domyślna kategoria pobiera surową historię zleceń i transakcji całego
rachunku udostępnianą przez terminal, niezależnie od listy zamknięć panelu.
Obejmuje także inne symbole, magic i przepływy pieniężne. Odczyt działa
w osobnym procesie i nie zmienia decyzji strategii. Zachowuje oryginalne
czasy, identyfikatory oraz pola kosztów. Eksport podaje zakres i wynik
pobierania; błąd, limit lub zmiana konta nie otrzymuje oznaczenia pełności.
Szczegółowe granice tego dowodu opisuje [LIVE_REPLAY.md](LIVE_REPLAY.md).

Pobieranie dużego allLogs zachowuje surowe bajty pliku i nie korzysta z dawnego
odczytu tekstowego ograniczonego do 32 MiB. Obejmuje wyłącznie ostatni
ukończony eksport. Lokalny serwer sprawdza dokładny Host i Origin, także
przed WebSocket; domena tylko przypominająca localhost jest odrzucana.
To ochrona przed żądaniami obcych stron, nie system uwierzytelniania lokalnych
programów. Interfejs korzysta z adresu pętli zwrotnej.

## English summary

GOD-X7 remains the primary preset with unchanged strategy settings. The
Drawdown card now includes daily Real Drawdown, calculated from starting equity
to the lowest qualified observed equity. Its tooltip shows the account-currency
amount and percentage. Missing daily history remains unavailable.

Backtests record daily RDD and full-observation maximum drawdown independently
of chart reduction. A corrected drawdown value can differ from a legacy report
without changing any trade or profit. Temporary accounting waits, uncertain
execution and broker refusals now have distinct diagnostics. Quote silence
recovery keeps its state across reconnects and uses increasing retry intervals;
confirmed transport failures retain their recovery path.

The follow-up records engine inputs, state and observed broker responses for
offline replay. Complete retained prefixes are included in allLogs by default.
Uncertain rearm opens are reconciled against strict order/account evidence,
counted once and assigned their original submission time for cooldown. Pending
reconciliation blocks new exposure while exits remain available; insufficient
proof requires review. The reconciliation state survives a same-account restart.

The default allLogs broker-history category adds available raw account orders
and deals, including other symbols and cash/cost events. A separate read-only
worker keeps long history requests off the trading sidecar queue. Raw broker
timestamps are preserved, and coverage, failures and partial results are
explicit. This is diagnostic material, not a retrospective input to trading.

Large allLogs downloads stream the exact completed export bytes instead of
using the old 32 MiB text reader. The local API now checks exact Host/Origin
values before REST and WebSocket handlers, rejecting localhost lookalikes.
This browser boundary is not authentication of other local processes.

The update provides Polish and English labels. Public packages remain free of
private credentials and Telegram sessions; private installation material stays
within the VPS package. Offline verification does not establish future trading
returns or recover events absent from the supplied history.
