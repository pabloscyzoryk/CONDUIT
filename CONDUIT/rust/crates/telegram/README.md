# conduit-telegram — klient MTProto

Odbiór sygnałów z kanałów Telegrama i zamiana ich na `IncomingMessage`
z `conduit-core`. Czysty Rust na `grammers-client` **0.10** — bez Telethona,
bez Pythona, bez API botowego.

```text
Telegram ──MTProto──▶ TelegramClient ──▶ IncomingMessage ──▶ Engine::on_message
```

Klient loguje się jako **konto użytkownika**, nie bot. Bot nie widzi wiadomości
w cudzych kanałach, a właśnie o nie tu chodzi.

## Konfiguracja

`api_id` i `api_hash` bierze się z <https://my.telegram.org> → *API development
tools*. Są przypisane do numeru telefonu dewelopera, ale logować się nimi może
każdy — nie trzeba wydawać osobnych par użytkownikom.

```rust
use conduit_telegram::{ClientConfig, TelegramClient, LoginStage, AsciiStyle};

let mut tg = TelegramClient::connect(ClientConfig {
    api_id: 1234567,
    api_hash: "…".into(),
    session_path: "conduit.session".into(),
    catch_up: false,          // patrz niżej — domyślnie WYŁĄCZONE i to jest celowe
    ignore_outgoing: true,
    queue_limit: 500,
}).await?;

if !tg.is_authorized().await? {
    let mut login = tg.qr_login();
    let mut stage = login.start().await?;
    loop {
        match stage {
            LoginStage::Qr(p) => {
                println!("{}", p.qr.to_ascii(AsciiStyle::Ansi));   // konsola
                std::fs::write("qr.svg", p.qr.to_svg(6))?;         // okno
                stage = login.step(tg.updates_mut()).await?;       // czeka na skan
            }
            LoginStage::PasswordRequired { hint } | LoginStage::PasswordWrong { hint } => {
                let pw = zapytaj_o_haslo(hint.as_deref());
                stage = login.submit_password(&pw).await?;
            }
            LoginStage::Done(me) => { println!("zalogowano: {}", me.name); break; }
        }
    }
}

tg.refresh_dialogs(0).await?;    // nazwy czatów + wykrycie forów + cache peerów
while let Ok(msg) = tg.next_message().await {
    engine.on_message(&mut broker, &msg);
}
```

## Logowanie kodem QR

`grammers` **nie ma** logowania QR — ma tylko kod z SMS-a i token bota.
Zostało dopisane od zera na surowych wywołaniach protokołu, zgodnie
z <https://core.telegram.org/api/qr-login>:

```text
auth.exportLoginToken ──▶ loginToken{expires, token}
                              └─▶ tg://login?token=… ──▶ [KOD QR]
updateLoginToken ◀── użytkownik skanuje oficjalną aplikacją ───┘
auth.exportLoginToken ──▶ loginTokenSuccess           → zalogowano
                       ├▶ loginTokenMigrateTo{dc}     → auth.importLoginToken w tym DC
                       └▶ SESSION_PASSWORD_NEEDED     → hasło 2FA (SRP)
```

Skanować trzeba **oficjalną aplikacją Telegrama**
(*Ustawienia → Urządzenia → Podłącz urządzenie*), nie dowolnym czytnikiem QR.

Dwie rzeczy, które decydują o tym, czy to działa w praktyce:

* **Token żyje ~30 s.** Obsługa wygaśnięcia nie jest dodatkiem, tylko głównym
  trybem pracy pętli — `step()` po wygaśnięciu zwraca nowy kod do pokazania.
* **Nie polegamy wyłącznie na `updateLoginToken`.** Przed zalogowaniem strumień
  aktualizacji jest wątły; gdyby ta aktualizacja zaginęła, logowanie zawisłoby
  na zawsze. Dlatego wygaśnięcie tokenu też wyzwala ponowne `exportLoginToken`
  — a jeśli użytkownik zdążył zeskanować, to wywołanie zwróci po prostu
  `loginTokenSuccess`. Zgubiona aktualizacja opóźnia logowanie o kilkanaście
  sekund zamiast je zrywać.

Hasło 2FA jest liczone protokołem **SRP** (`auth.checkPassword`) — serwer nigdy
nie dostaje hasła jawnie. Błędne hasło daje `LoginStage::PasswordWrong` i można
spróbować ponownie bez powtarzania całego logowania.

### Kod QR w dwóch postaciach

`QrRender` oddaje macierz modułów oraz:

* `to_svg(px)` — do okna. **Ma jawne białe tło**: przezroczysty SVG na ciemnym
  motywie daje kod odwrócony, którego część telefonów nie przeczyta.
* `to_ascii(style)` — do konsoli:
  * `Ansi` — kolory tła, wygląda poprawnie **przy każdym motywie terminala** (zalecane),
  * `Blocks` / `BlocksInverted` — pełne bloki dla jasnego / ciemnego tła,
  * `HalfBlocks` — dwa razy niższy obrazek (`▀▄█`).

Strefa ciszy to 4 moduły — mniej naprawdę bywa nieczytelne dla telefonu
trzymanego pod kątem.

## Sesja

`grammers-session` 0.10 daje tylko `MemorySession` (znika przy zamknięciu)
i `SqliteSession` (ciągnie `libsql`). Tutaj jest trzeci wariant — `FileSession`:
jeden plik JSON, zapis atomowy (plik obok + podmiana nazwy), plus przenośny
`session_string` (base64).

To nie jest wygoda, tylko konieczność: logowanie do Telegrama jest drogie,
kilka nieudanych prób pod rząd kończy się blokadą na godziny. Sesja, która nie
przeżywa restartu, to bot, który po kilku restartach nie może się zalogować.

> **Plik sesji jest równoważny zalogowanemu urządzeniu.** Kto go ma, ma dostęp
> do konta. Traktować jak hasło; `Debug` celowo nie wypisuje jego zawartości.

## Odbiór: edycje, odpowiedzi, tematy

To jest najtrudniejsza część crate'a i główny powód, dla którego moduł
`incoming` jest osobny i gęsto otestowany.

**Edycje.** W kanale ATFX co trzeci sygnał bywa poprawiany po wysłaniu.
`UpdateEditMessage` niesie tę samą wiadomość z tym samym `id`; potraktowana jak
nowa, otwiera drugi koszyk na ten sam sygnał. Dlatego `edit_of` niesie
identyfikator edytowanej wiadomości i silnik poprawia istniejący koszyk.
Rozstrzyga **rodzaj aktualizacji**, nie pole `edit_date` — Telegram ustawia je
także po doklejeniu podglądu odnośnika, więc dawałoby fałszywe edycje.

**Odpowiedzi.** Komunikaty zarządzające („zamknij połowę", „SL na BE") to
zwykle odpowiedzi na wiadomość z sygnałem — bez `reply_to` nie da się ich
przypisać do koszyka.

**Tematy forum — i pułapka, w którą wpada się zawsze.** W grupie z tematami
pole `reply_to_msg_id` ma **dwa różne znaczenia**:

| sytuacja | `forum_topic` | `reply_to_top_id` | `reply_to_msg_id` | wynik |
|---|---|---|---|---|
| zwykły kanał | `false` | — | wiadomość | odpowiedź |
| wprost do tematu | `true` | brak | **numer tematu** | temat, **bez odpowiedzi** |
| odpowiedź w temacie | `true` | numer tematu | wiadomość | temat + odpowiedź |

Zignorowanie drugiego wiersza daje bota, który każdą wiadomość w temacie uważa
za odpowiedź na wiadomość o numerze tematu — i przypina sygnały do przypadkowych
koszyków.

Temat trafia do `SourceKey.topic_id`, czyli jest **niezależnym źródłem**:
własne koszyki, własny preset.

Temat „Ogólny" (id 1) nie ma nagłówka odpowiedzi w ogóle. Nie da się go poznać
z samej aktualizacji — dlatego `refresh_dialogs()` zapamiętuje, które czaty są
forum, a `with_forum_default()` domyka brakujący temat na 1.

## Dlaczego `catch_up` jest domyślnie wyłączone

Sygnał sprzed godzin jest nie tylko bezwartościowy — jest **groźny**: bot
otworzyłby koszyk na cenę, której już dawno nie ma. Włączać świadomie.

## Lista kanałów i tematów

`list_dialogs()` i `list_topics()` służą do wyboru źródeł w interfejsie, ale
mają też dwie techniczne role: napełniają cache peerów w sesji (bez niego
nadrabianie zaległych aktualizacji nie działa — mówi o tym wprost dokumentacja
`stream_updates`) i wykrywają, które czaty są forum.

## Powiadomienia

```rust
tg.notify(chat_id, Some(topic_id), "koszyk 12: TP1").await?;
```
W grupie z tematami wiadomość bez `topic_id` wyląduje w „Ogólnym", a nie tam,
gdzie leci sygnał.

## Testy

```
cargo test -p conduit-telegram      # 47 testów, bez sieci i bez poświadczeń
```

Pokrywają:
* **rozkład nagłówka odpowiedzi** — wszystkie trzy przypadki z tabeli wyżej,
  plus odpowiedź na relację i brak nagłówka;
* budowę `IncomingMessage` z prawdziwych struktur `tl::types::Message`
  (nie z atrap): nowa wiadomość, edycja, odpowiedź, temat, temat „Ogólny",
  własne echo, identyfikatory kanału/grupy w konwencji Bot API;
* kodowanie tokenu do `tg://login?token=…` (base64url bez dopełnienia)
  w obie strony;
* generowanie kodu QR: wymiary, **znacznik pozycjonujący**, pusta strefa ciszy
  ze wszystkich stron, poprawność SVG (jasne tło, sklejanie prostokątów),
  wszystkie style ASCII/ANSI;
* sesję: zapis/odczyt, nadpisanie istniejącego pliku (pułapka Windows),
  `session_string`, odrzucenie pliku uszkodzonego i z obcej wersji formatu,
  brak duplikatów stanu kanałów, oraz **wyszukanie wpisu „o sobie"**.

Ten ostatni test złapał realny błąd: `PeerId::self_user()` to **wartownik**,
którego nie ma w mapie peerów — wpis leży pod prawdziwym numerem użytkownika,
z flagą `is_self`. Zwykłe `get()` (tak robi `MemorySession` z biblioteki)
zwraca `None`, przez co `stream_updates` uznaje, że nie jesteśmy zalogowani,
i nigdy nie nadrabia zaległości. `FileSession` szuka po fladze — tak jak
magazyn SQLite.

## CZEGO NIE UDAŁO SIĘ ZWERYFIKOWAĆ

Nie miałem `api_id`/`api_hash` ani konta Telegrama. **Niesprawdzone na żywym
połączeniu jest wszystko, co wymaga serwera:**

* **całe logowanie QR od strony sieci** — `auth.exportLoginToken`,
  odbiór `updateLoginToken`, `loginTokenSuccess`. Struktury i nazwy wywołań
  sprawdziłem w wygenerowanym kodzie `grammers-tl-types` 0.10 (nie zgadywałem),
  ale **ani jedno z tych wywołań nie zostało wykonane**;
* **przeniesienie do innego datacentrum** (`loginTokenMigrateTo` →
  `auth.importLoginToken`) — ścieżka najrzadsza i najbardziej podatna na błąd,
  całkowicie niesprawdzona. Dodatkowo `grammers` nie udostępnia publicznie
  `disconnect_from_dc`, więc stare połączenie zostaje otwarte do czasu
  wygaszenia puli;
* **hasło 2FA** — `check_password` z biblioteki jest wywoływane poprawnie
  co do typów, ale bez konta z włączonym 2FA nie dało się przejść tej ścieżki;
* **czy `updateLoginToken` faktycznie przechodzi przez `UpdateStream`** przed
  zalogowaniem. Jeśli `MessageBoxes` je odfiltruje, logowanie i tak się uda —
  ale dopiero po wygaśnięciu tokenu (kilkanaście sekund zamiast natychmiast).
  To jest świadomie wbudowana ścieżka zapasowa, nie przypadek;
* **odbiór na żywo**: `list_dialogs`, `list_topics`, `notify` oraz zachowanie
  `stream_updates` przy zerwaniu połączenia. Konwersja aktualizacji jest
  otestowana na prawdziwych typach TL, ale **nie na prawdziwym ruchu**;
* mapowanie tematów na `topic_id` sprawdzono na ręcznie złożonych nagłówkach
  zgodnych ze schematem TL — **nie na wiadomościach z realnej grupy forum**.
  Zachowanie tematu „Ogólny" (czy przychodzi bez nagłówka, czy z
  `reply_to_msg_id = 1`) potrafi zależeć od wersji klienta i wymaga sprawdzenia
  na kanale ATFX;
* nie sprawdziłem, jak zachowuje się limit kolejki aktualizacji przy zalewie
  wiadomości ani jak często realnie występuje `flood wait`.

Zweryfikowane bez poświadczeń jest natomiast wszystko, co wymieniono w sekcji
„Testy" — w tym cała logika, na której najłatwiej się przewrócić: edycje,
odpowiedzi i tematy.
