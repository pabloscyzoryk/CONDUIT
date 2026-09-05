import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { QrCode } from "@/components/auth/QrCode";
import { Button, Icon } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { useT, RichT } from "@/i18n";
import { useTheme } from "@/store/useTheme";
import { api, type AuthState } from "@/store/transport";
import "./login.css";

/* ============================================================
   LOGOWANIE

   Dwa tryby, jeden ekran:

   * BACKEND DZIAŁA — kod QR jest PRAWDZIWY: przychodzi z serwera jako SVG
     wygenerowany z tokenu logowania, a pole 2FA pojawia się dokładnie wtedy,
     gdy serwer zgłosi `waitingPassword`. Nic tu nie jest zgadywane po stronie
     przeglądarki; UI odzwierciedla automat stanu z `crates/server/src/auth.rs`.

   * BRAK BACKENDU — zostaje symulacja z prototypu, żeby dało się obejrzeć
     interfejs bez uruchamiania czegokolwiek.

   Ekran NIGDY nie udaje udanego logowania. Gdy serwer mówi, że klient MTProto
   nie jest podłączony, jest to napisane wprost, a wejście do terminala wymaga
   świadomego kliknięcia w osobny, opisany przycisk.
   ============================================================ */

type DemoPhase = "idle" | "scanning" | "twofa" | "confirm" | "done";

const STEPS = ["login.step.1", "login.step.2", "login.step.3"];

/** Kroki zdobycia api_id/api_hash — pierwsze uruchomienie, raz na zawsze. */
const KROKI_API = ["login.api.step.1", "login.api.step.2", "login.api.step.3"];

/** Cztery tryby w lewej kolumnie. Tytuł jest NAZWĄ TRYBU (MANUAL/AUTO/AUTO-EA/AI
 *  to identyfikatory widoczne w całym panelu — nie tłumaczymy), opis w słowniku. */
const MODES = [
  { icon: "hand" as const, title: "MANUAL", text: "login.mode.manual" },
  { icon: "bolt" as const, title: "AUTO", text: "login.mode.auto" },
  { icon: "robot" as const, title: "AUTO-EA", text: "login.mode.autoea" },
  { icon: "brain" as const, title: "AI", text: "login.mode.ai" },
];

/** Co ile odpytywać serwer o stan logowania. */
const POLL_MS = 1500;

export function LoginView() {
  const app = useApp();
  const { theme, toggle } = useTheme();

  /* backend jest, jeśli store dostał od niego cokolwiek o logowaniu */
  const maBackend = app.auth !== null;

  return (
    <Rama theme={theme} toggle={toggle}>
      {maBackend ? <AuthPrawdziwe /> : <AuthSymulowane />}
    </Rama>
  );
}

/* ------------------------------------------------------------------
   Wspólna oprawa: tło, lewa kolumna marki, stopka.
   ------------------------------------------------------------------ */
function Rama({ theme, toggle, children }: { theme: string; toggle: () => void; children: ReactNode }) {
  const t = useT();
  return (
    <div className="login">
      <div className="login__bg" aria-hidden="true">
        <span className="login__orb login__orb--1" />
        <span className="login__orb login__orb--2" />
        <span className="login__orb login__orb--3" />
        <span className="login__grid" />
      </div>

      <button className="login__theme" onClick={toggle} title={t("login.theme")}>
        <Icon name={theme === "dark" ? "sun" : "moon"} size={16} />
      </button>

      <div className="login__panel">
        <section className="login__brand">
          <div className="login__logo">
            <svg viewBox="0 0 32 32" width="34" height="34" aria-hidden="true">
              <rect width="32" height="32" rx="9" fill="url(#lg)" />
              <path
                d="M8 20.5l5.5-7 3.8 4.6L24 9"
                stroke="#fff"
                strokeWidth="2.4"
                fill="none"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <circle cx="24" cy="9" r="2.6" fill="#fff" />
              <defs>
                <linearGradient id="lg" x1="0" y1="0" x2="32" y2="32">
                  <stop stopColor="var(--accent-hover)" />
                  <stop offset="1" stopColor="var(--accent)" />
                </linearGradient>
              </defs>
            </svg>
            <div>
              <h1>CONDUIT</h1>
              <p>Telegram → MetaTrader 5</p>
            </div>
          </div>

          <h2 className="login__headline">
            {t("login.headline.1")}
            <br />
            <span>{t("login.headline.2")}</span>
          </h2>

          <p className="login__lede">{t("login.lede")}</p>

          <ul className="login__features">
            {MODES.map((f) => (
              <li key={f.title}>
                <span className="login__ficon">
                  <Icon name={f.icon} size={15} />
                </span>
                <div>
                  <b>{f.title}</b>
                  <span>{t(f.text)}</span>
                </div>
              </li>
            ))}
          </ul>
        </section>

        {children}
      </div>

      <footer className="login__foot">
        <span>CONDUIT · Telegram → MetaTrader 5</span>
        <span>MetaTrader 5 · XAUUSD</span>
      </footer>
    </div>
  );
}

/* ------------------------------------------------------------------
   WARIANT Z BACKENDEM — prawdziwy kod QR i prawdziwe 2FA.
   ------------------------------------------------------------------ */
function AuthPrawdziwe() {
  const app = useApp();
  const t = useT();
  const [stan, setStan] = useState<AuthState>(app.auth ?? { stage: "loggedOut" });
  const [pass, setPass] = useState("");
  const [passError, setPassError] = useState("");
  const [showPass, setShowPass] = useState(false);
  const [wysylam, setWysylam] = useState(false);
  const [ttl, setTtl] = useState(0);
  const startowano = useRef(false);

  const stage = stan.stage;

  /* --- start: generujemy token od razu po wejściu na ekran --- */
  const start = useCallback(async () => {
    const s = await app.startQrLogin();
    if (s) setStan(s);
  }, [app]);

  useEffect(() => {
    if (startowano.current) return;
    /* Bez api_id/api_hash NIE MA czego generować: token logowania wydaje
       serwer Telegrama w odpowiedzi na `auth.exportLoginToken`, a to wywołanie
       wymaga poświadczeń aplikacji. Dopóki ich nie ma, pokazujemy formularz —
       nie atrapę kodu. Etap `connecting` też pomijamy: tam serwer właśnie
       wznawia zapisaną sesję i sam wyda kod, gdy będzie trzeba. */
    if (stage === "needCredentials" || stage === "connecting" || stage === "loggedIn") return;
    startowano.current = true;
    void start();
  }, [start, stage]);

  /* --- polling stanu; kończymy, gdy nie ma już czego pilnować --- */
  useEffect(() => {
    if (stage === "loggedIn") return;
    const h = window.setInterval(() => {
      void api
        .authState()
        .then(setStan)
        .catch(() => undefined);
    }, POLL_MS);
    return () => window.clearInterval(h);
  }, [stage]);

  /* --- odliczanie do wygaśnięcia tokenu + samoodnowienie --- */
  const expiresAt = stan.expiresAt;
  useEffect(() => {
    if (stage !== "waitingScan" || !expiresAt) {
      setTtl(0);
      return;
    }
    const h = window.setInterval(() => {
      const left = Math.max(0, Math.round((expiresAt - Date.now()) / 1000));
      setTtl(left);
      if (left === 0) void start();
    }, 1000);
    return () => window.clearInterval(h);
  }, [stage, expiresAt, start]);

  /* --- zalogowani: wchodzimy do terminala --- */
  useEffect(() => {
    if (stage === "loggedIn") app.login();
    // celowo bez `app` w zależnościach — obiekt kontekstu zmienia się co deltę
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stage]);

  const submitPass = async () => {
    if (!pass.trim()) {
      setPassError(t("login.2fa.required"));
      return;
    }
    setPassError("");
    setWysylam(true);
    const s = await app.submit2fa(pass);
    setWysylam(false);
    if (!s) return;
    if (s.stage === "error") {
      setPassError(s.error ?? t("login.2fa.rejected"));
      return;
    }
    setPass("");
    setStan(s);
  };

  /* Serwer zgłasza błąd przy aktywnym kodzie = klient MTProto nie jest
     podłączony (patrz `UnconfiguredAuth`). Mówimy to wprost. */
  const klientNiepodlaczony = stage === "waitingScan" && !!stan.error;

  /* ETAP ZERO: nie ma api_id/api_hash — pytamy o nie, zanim cokolwiek
     narysujemy. To jedyny ekran, na którym użytkownik coś przepisuje;
     potem dane siedzą w secrets.json i logowanie idzie samo. */
  if (stage === "needCredentials") {
    /* `startowano` ustawiamy TU, a nie w efekcie: serwer po przyjęciu
       poświadczeń od razu oddaje pierwszy kod QR. Bez tego efekt zobaczyłby
       świeży etap `waitingScan` i wywołał `exportLoginToken` drugi raz —
       token z formularza zostałby zastąpiony sekundę po narysowaniu, a
       Telegram liczy te wywołania i blokuje przy nadmiarze. */
    return (
      <FormularzPoswiadczen
        stan={stan}
        onOk={(s) => {
          startowano.current = true;
          setStan(s);
        }}
      />
    );
  }

  return (
    <section className="login__auth">
      <div className="login__auth-head">
        <span className="badge badge--accent badge--dot">MTProto</span>
        <h3>{t("login.qr.title")}</h3>
        <p>{t("login.qr.subtitle")}</p>
      </div>

      <div className={`qr-stage qr-stage--${stage}`}>
        <div className="qr-wrap" aria-label={t("login.qr.aria")}>
          {stan.qrSvg ? (
            /* SVG z serwera: ciemne moduły mają `currentColor`, tło jest
               przezroczyste — dzięki temu kod dziedziczy motyw i paletę */
            <span
              className="qr-svg"
              style={{ color: "var(--text)", display: "block", width: 236, height: 236 }}
              dangerouslySetInnerHTML={{ __html: stan.qrSvg }}
            />
          ) : (
            <QrCode seed={1} size={236} />
          )}

          {}

          {stage === "waitingScan" && !klientNiepodlaczony && <span className="qr-scanline" />}

          {stage === "confirming" && (
            <span className="qr-overlay">
              <Icon name="user" size={26} />
              <b>{t("login.confirmOnPhone")}</b>
              <span>{stan.user ?? t("login.newDevice")}</span>
            </span>
          )}

          {stage === "expired" && (
            <span className="qr-overlay">
              <Icon name="alert" size={24} />
              <b>{t("login.expired")}</b>
              <span>{t("login.expired.text")}</span>
            </span>
          )}

          {stage === "connecting" && (
            <span className="qr-overlay">
              <span className="qr-spinner" />
              <b>{t("login.connecting")}</b>
              <span>{stan.sessionSaved ? t("login.connecting.resume") : t("login.connecting.fresh")}</span>
            </span>
          )}

          {stage === "loggedIn" && (
            <span className="qr-overlay qr-overlay--ok">
              <span className="qr-check">
                <Icon name="check" size={26} strokeWidth={3} />
              </span>
              <b>{t("login.loggedIn")}</b>
              <span>{t("login.loadingTerminal")}</span>
            </span>
          )}
        </div>

        {stage === "waitingScan" && ttl > 0 && (
          <div className="qr-ttl">
            <RichT k="login.ttl" vars={{ n: ttl }} />
          </div>
        )}
      </div>

      {stage === "waitingPassword" ? (
        <div className="twofa" role="group" aria-label={t("login.2fa.title")}>
          <div className="twofa__head">
            <span className="twofa__icon">
              <Icon name="lock" size={16} />
            </span>
            <div>
              <b>{t("login.2fa.title")}</b>
              <span>
                {stan.passwordHint ? t("login.2fa.hint", { v: stan.passwordHint }) : t("login.2fa.cloudPass")}
              </span>
            </div>
          </div>

          <label className="field__label" htmlFor="tfa">
            {t("login.2fa.password")}
          </label>
          <div className="twofa__row">
            <input
              id="tfa"
              className="input"
              type={showPass ? "text" : "password"}
              value={pass}
              autoFocus
              autoComplete="current-password"
              placeholder={t("login.2fa.placeholder")}
              onChange={(e) => {
                setPass(e.target.value);
                setPassError("");
              }}
              onKeyDown={(e) => e.key === "Enter" && void submitPass()}
            />
            <Button
              variant="ghost"
              icon={showPass ? "eye-off" : "eye"}
              onClick={() => setShowPass((s) => !s)}
              title={t("login.2fa.toggle")}
            />
          </div>

          {passError && (
            <span className="twofa__err">
              <Icon name="alert" size={12} /> {passError}
            </span>
          )}

          <div className="twofa__actions">
            <Button variant="primary" icon="check" block disabled={wysylam} onClick={() => void submitPass()}>
              {wysylam ? t("login.2fa.checking") : t("login.2fa.confirm")}
            </Button>
            <Button variant="ghost" disabled={wysylam} onClick={() => void start()}>
              {t("login.2fa.restart")}
            </Button>
          </div>

          <span className="hint">{t("login.2fa.note")}</span>
        </div>
      ) : (
        <ol className="login__steps">
          {STEPS.map((k, i) => (
            <li key={k}>
              <span>{i + 1}</span>
              {t(k)}
            </li>
          ))}
        </ol>
      )}

      {stage === "error" && (
        <div className="twofa__err" style={{ marginTop: 10 }}>
          <Icon name="alert" size={12} /> {stan.error ?? t("login.failed")}
        </div>
      )}

      <div className="login__alt">
        <button className="login__link" onClick={() => void start()}>
          <Icon name="bolt" size={13} /> {t("login.newCode")}
        </button>
        <span className="divider--v" />
        <button
          className="login__link"
          title={t("login.changeApi.title")}
          onClick={() => {
            void app.forgetTelegramCredentials().then((s) => {
              if (s) setStan(s);
              startowano.current = false;
            });
          }}
        >
          <Icon name="refresh" size={13} /> {t("login.changeApi")}
        </button>
        <span className="divider--v" />
        <span className="hint">
          {t("login.server")} <b>{app.backendStatus === "open" ? t("login.server.open") : app.backendStatus}</b>
        </span>
      </div>

      {klientNiepodlaczony && (
        <div className="twofa" style={{ marginTop: 12 }}>
          <div className="twofa__head">
            <span className="twofa__icon">
              <Icon name="alert" size={16} />
            </span>
            <div>
              <b>{t("login.noClient")}</b>
              <span>{stan.error}</span>
            </div>
          </div>
          <span className="hint">{t("login.noClient.text")}</span>
          <div className="twofa__actions">
            <Button variant="ghost" icon="cursor" block onClick={app.login}>
              {t("login.noClient.enter")}
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

/* ------------------------------------------------------------------
   ETAP ZERO — api_id i api_hash z my.telegram.org.

   Dlaczego to w ogóle istnieje: Telegram wydaje token logowania (ten, który
   koduje kod QR) dopiero w odpowiedzi na `auth.exportLoginToken`, a to
   wywołanie wymaga poświadczeń APLIKACJI. Kod QR narysowany przed ich
   podaniem może zawierać wyłącznie wymyślony adres — telefon go zeskanuje
   i nic się nie stanie. Dlatego najpierw formularz, potem prawdziwy kod.

   Użytkownik przechodzi tędy RAZ: po udanym logowaniu api_id, api_hash
   i łańcuch sesji lądują w `secrets.json` i kolejne starty logują się same.
   ------------------------------------------------------------------ */
function FormularzPoswiadczen({
  stan,
  onOk,
}: {
  stan: AuthState;
  onOk: (s: AuthState) => void;
}) {
  const app = useApp();
  const t = useT();
  const [apiId, setApiId] = useState(stan.apiId ? String(stan.apiId) : "");
  const [apiHash, setApiHash] = useState("");
  const [pokazHash, setPokazHash] = useState(false);
  const [blad, setBlad] = useState("");
  const [wysylam, setWysylam] = useState(false);

  /* Walidacja lokalna, zanim ruszymy sieć: api_hash z my.telegram.org ma
     dokładnie 32 znaki szesnastkowe. Bez tego sprawdzenia literówka wraca
     jako odmowa serwera Telegrama po kilkunastu sekundach i bez wskazówki. */
  const idOk = /^\d+$/.test(apiId.trim()) && Number(apiId.trim()) > 0;
  const hashOk = /^[0-9a-fA-F]{32}$/.test(apiHash.trim());
  const moznaWyslac = idOk && hashOk && !wysylam;

  const wyslij = async () => {
    if (!idOk) {
      setBlad(t("login.api.err.id"));
      return;
    }
    if (!hashOk) {
      setBlad(t("login.api.err.hash", { n: apiHash.trim().length }));
      return;
    }
    setBlad("");
    setWysylam(true);
    const s = await app.setTelegramCredentials(apiId, apiHash);
    setWysylam(false);
    if (!s) return;
    if (s.stage === "error") {
      setBlad(s.error ?? t("login.api.err.rejected"));
      return;
    }
    // hasz znika z pamięci przeglądarki, gdy tylko trafi na serwer
    setApiHash("");
    onOk(s);
  };

  return (
    <section className="login__auth">
      <div className="login__auth-head">
        <span className="badge badge--accent badge--dot">{t("login.api.badge")}</span>
        <h3>{t("login.api.title")}</h3>
        <p>
          <RichT k="login.api.intro" />
        </p>
      </div>

      <ol className="login__steps">
        {KROKI_API.map((k, i) => (
          <li key={k}>
            <span>{i + 1}</span>
            {t(k)}
          </li>
        ))}
      </ol>

      <div className="twofa" role="group" aria-label={t("login.api.aria")}>
        <label className="field__label" htmlFor="api-id">
          api_id
        </label>
        <input
          id="api-id"
          className="input"
          value={apiId}
          inputMode="numeric"
          autoFocus
          placeholder={t("login.api.idPlaceholder")}
          onChange={(e) => {
            setApiId(e.target.value);
            setBlad("");
          }}
          onKeyDown={(e) => e.key === "Enter" && void wyslij()}
        />

        <label className="field__label" htmlFor="api-hash" style={{ marginTop: 10 }}>
          api_hash
        </label>
        <div className="twofa__row">
          <input
            id="api-hash"
            className="input"
            type={pokazHash ? "text" : "password"}
            value={apiHash}
            autoComplete="off"
            spellCheck={false}
            placeholder={t("login.api.hashPlaceholder")}
            onChange={(e) => {
              setApiHash(e.target.value);
              setBlad("");
            }}
            onKeyDown={(e) => e.key === "Enter" && void wyslij()}
          />
          <Button
            variant="ghost"
            icon={pokazHash ? "eye-off" : "eye"}
            onClick={() => setPokazHash((s) => !s)}
            title={t("login.api.toggleHash")}
          />
        </div>

        {blad && (
          <span className="twofa__err">
            <Icon name="alert" size={12} /> {blad}
          </span>
        )}

        <div className="twofa__actions">
          <Button
            variant="primary"
            icon="check"
            block
            disabled={!moznaWyslac}
            onClick={() => void wyslij()}
          >
            {wysylam ? t("login.api.sending") : t("login.api.submit")}
          </Button>
        </div>

        <span className="hint">
          <RichT k="login.api.note" />
        </span>
      </div>

      <div className="login__alt">
        <a
          className="login__link"
          href="https://my.telegram.org/apps"
          target="_blank"
          rel="noreferrer noopener"
        >
          <Icon name="bolt" size={13} /> {t("login.api.open")}
        </a>
        <span className="divider--v" />
        <span className="hint">
          {t("login.server")} <b>{app.backendStatus === "open" ? t("login.server.open") : app.backendStatus}</b>
        </span>
      </div>

      {stan.error && (
        <div className="twofa__err" style={{ marginTop: 10 }}>
          <Icon name="alert" size={12} /> {stan.error}
        </div>
      )}
    </section>
  );
}

/* ------------------------------------------------------------------
   WARIANT BEZ BACKENDU — zachowanie prototypu, jawnie oznaczone.
   ------------------------------------------------------------------ */
function AuthSymulowane() {
  const { login } = useApp();
  const t = useT();
  const [phase, setPhase] = useState<DemoPhase>("idle");
  const [progress, setProgress] = useState(0);
  const [seed, setSeed] = useState(() => Math.floor(Math.random() * 1e6));
  const [ttl, setTtl] = useState(58);
  const [pass, setPass] = useState("");
  const [passError, setPassError] = useState("");
  const [showPass, setShowPass] = useState(false);
  const timers = useRef<number[]>([]);

  useEffect(() => {
    if (phase !== "idle") return;
    const h = window.setInterval(() => {
      setTtl((t) => {
        if (t <= 1) {
          setSeed(Math.floor(Math.random() * 1e6));
          return 58;
        }
        return t - 1;
      });
    }, 1000);
    return () => window.clearInterval(h);
  }, [phase]);

  useEffect(() => () => timers.current.forEach((t) => window.clearTimeout(t)), []);

  const finish = () => {
    setPhase("confirm");
    timers.current.push(
      window.setTimeout(() => {
        setPhase("done");
        timers.current.push(window.setTimeout(login, 900));
      }, 1150),
    );
  };

  const start = () => {
    if (phase !== "idle") return;
    setPhase("scanning");
    setProgress(0);
    let p = 0;
    const step = window.setInterval(() => {
      p += 4 + Math.random() * 9;
      setProgress(Math.min(100, p));
      if (p >= 100) {
        window.clearInterval(step);
        // Konto z 2FA prosi o hasło chmury. Losujemy 50/50, żeby oba
        // warianty dało się obejrzeć bez backendu.
        if (Math.random() < 0.5) setPhase("twofa");
        else finish();
      }
    }, 90);
    timers.current.push(step);
  };

  const submitPass = () => {
    if (!pass.trim()) {
      setPassError(t("login.2fa.required"));
      return;
    }
    setPassError("");
    setPhase("scanning");
    setProgress(100);
    timers.current.push(window.setTimeout(finish, 700));
  };

  return (
    <section className="login__auth">
      <div className="login__auth-head">
        <span className="badge badge--warn badge--dot">{t("login.demo.badge")}</span>
        <h3>{t("login.qr.title")}</h3>
        <p>
          <RichT k="login.demo.subtitle" />
        </p>
      </div>

      <div className={`qr-stage qr-stage--${phase}`}>
        <button
          className="qr-wrap"
          onClick={start}
          disabled={phase !== "idle"}
          aria-label={t("login.demo.aria")}
        >
          <QrCode seed={seed} size={236} />

          {phase === "idle" && <span className="qr-scanline" />}

          {phase === "idle" && (
            <span className="qr-hint">
              <Icon name="cursor" size={13} />
              {t("login.demo.clickCode")}
            </span>
          )}

          {phase === "scanning" && (
            <span className="qr-overlay">
              <span className="qr-spinner" />
              <b>{t("login.demo.scanning")}</b>
              <span className="qr-progress">
                <span style={{ width: `${progress}%` }} />
              </span>
            </span>
          )}

          {phase === "confirm" && (
            <span className="qr-overlay">
              <Icon name="user" size={26} />
              <b>{t("login.confirmOnPhone")}</b>
              <span>Demo User · mobile device</span>
            </span>
          )}

          {phase === "done" && (
            <span className="qr-overlay qr-overlay--ok">
              <span className="qr-check">
                <Icon name="check" size={26} strokeWidth={3} />
              </span>
              <b>{t("login.loggedIn")}</b>
              <span>{t("login.loadingTerminal")}</span>
            </span>
          )}
        </button>

        {phase === "idle" && (
          <div className="qr-ttl">
            <RichT k="login.ttl" vars={{ n: ttl }} />
          </div>
        )}
      </div>

      {phase === "twofa" ? (
        <div className="twofa" role="group" aria-label={t("login.2fa.title")}>
          <div className="twofa__head">
            <span className="twofa__icon">
              <Icon name="lock" size={16} />
            </span>
            <div>
              <b>{t("login.2fa.title")}</b>
              <span>{t("login.2fa.cloudPass")}</span>
            </div>
          </div>

          <label className="field__label" htmlFor="tfa">
            {t("login.2fa.password")}
          </label>
          <div className="twofa__row">
            <input
              id="tfa"
              className="input"
              type={showPass ? "text" : "password"}
              value={pass}
              autoFocus
              placeholder={t("login.2fa.placeholder")}
              onChange={(e) => {
                setPass(e.target.value);
                setPassError("");
              }}
              onKeyDown={(e) => e.key === "Enter" && submitPass()}
            />
            <Button
              variant="ghost"
              icon={showPass ? "eye-off" : "eye"}
              onClick={() => setShowPass((s) => !s)}
              title={t("login.2fa.toggle")}
            />
          </div>

          {passError && (
            <span className="twofa__err">
              <Icon name="alert" size={12} /> {passError}
            </span>
          )}

          <div className="twofa__actions">
            <Button variant="primary" icon="check" block onClick={submitPass}>
              {t("login.2fa.confirm")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => {
                setPhase("idle");
                setPass("");
                setPassError("");
              }}
            >
              {t("common.cancel")}
            </Button>
          </div>

          <span className="hint">{t("login.demo.anyPass")}</span>
        </div>
      ) : (
        <ol className="login__steps">
          {STEPS.map((k, i) => (
            <li key={k}>
              <span>{i + 1}</span>
              {t(k)}
            </li>
          ))}
        </ol>
      )}

      <div className="login__alt">
        <button className="login__link" onClick={start} disabled={phase !== "idle"}>
          <Icon name="bolt" size={13} /> {t("login.demo.byPhone")}
        </button>
        <span className="divider--v" />
        <span className="hint">{t("login.demo.noBackend")}</span>
      </div>

      <PomijanieTelegrama />
    </section>
  );
}


function PomijanieTelegrama() {
  const app = useApp();
  const t = useT();
  // Gdy ekran otwarto RĘCZNIE z panelu, wychodzimy z niego z powrotem do
  // terminala. Gdy to pierwsze wejście — wpuszczamy do terminala bez sesji.
  const wroc = () => (app.pokazLogowanieTg ? app.zamknijLogowanieTg() : app.wejdzBezTelegrama());

  return (
    <div className="login__alt" style={{ marginTop: "var(--sp-3)" }}>
      <Button variant="outline" icon="arrow-up-right" onClick={wroc}>
        {app.pokazLogowanieTg ? t("login.skip.back") : t("login.skip.continue")}
      </Button>
      <span className="hint">{t("login.skip.note")}</span>
    </div>
  );
}
