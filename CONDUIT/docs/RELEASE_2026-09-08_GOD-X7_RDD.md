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

The update provides Polish and English labels. Public packages remain free of
private credentials and Telegram sessions; private installation material stays
within the VPS package. Offline verification does not establish future trading
returns or recover events absent from the supplied history.
