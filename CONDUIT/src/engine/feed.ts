import { CHANNELS } from "@/data/telegram";
import type { Basket } from "@/types";

/* ============================================================
   GENERATOR STRUMIENIA TELEGRAMA
   Produkuje realistyczne wiadomosci kanalu sygnalowego w formacie
   ATFX — wejscia w okolicy biezacej ceny oraz komunikaty
   zarzadzajace do zywych koszykow.
   ============================================================ */

export interface FeedMessage {
  channelId: number;
  channelName: string;
  topicName?: string;
  text: string;
  replyToBasket?: number;
}

const R = (a: number, b: number) => a + Math.random() * (b - a);
const pick = <T,>(arr: T[]): T => arr[Math.floor(Math.random() * arr.length)];

const TAGS = [
  "HIGH RISK TRADE",
  "FIRST ENTRY CAN BE 3 PIPS BELOW",
  "MANAGE YOUR RISK",
  "MAY NOT BE AROUND FOR MANAGEMENT",
  "SCALP SETUP",
  "",
  "",
];

const CHATTER = [
  "SYNTHETIC CHATTER: WAITING FOR A VALID SETUP.",
  "SYNTHETIC CHATTER: VOLATILITY IS ELEVATED.",
  "SYNTHETIC CHATTER: NO NEW ENTRY YET.",
  "SYNTHETIC CHATTER: REVIEW RISK BEFORE TRADING.",
  "SYNTHETIC CHATTER: MARKET CONDITIONS ARE UNCLEAR.",
];

const ATFX = CHANNELS[0];
const RM = CHANNELS[1];
const FORUM = CHANNELS[2];

/** Nowy sygnal wejscia w okolicy biezacej ceny. */
export function makeEntryMessage(price: number): FeedMessage {
  const buy = Math.random() < 0.62;
  const limit = Math.random() < 0.75;

  const zoneW = R(4, 9);
  const off = limit ? R(2.5, 7) : R(0, 1.5);

  let hi: number;
  let lo: number;
  if (buy) {
    hi = price - off;
    lo = hi - zoneW;
  } else {
    lo = price + off;
    hi = lo + zoneW;
  }

  const mid = (hi + lo) / 2;
  const slDist = R(9, 16);
  const sl = buy ? lo - slDist : hi + slDist;

  const tpStep = R(3.5, 6.5);
  const tps = [1, 2, 3].map((i) => (buy ? mid + tpStep * i : mid - tpStep * i));

  const f = (v: number) => v.toFixed(2);
  const dirWord = buy ? "BUY" : "SELL";
  const kindWord = limit ? `${dirWord} LIMITS` : dirWord;
  const emoji = buy ? "🟢" : "🔴";
  const tag = pick(TAGS);

  const useForum = Math.random() < 0.3;
  const src = useForum ? FORUM : ATFX;

  const text =
    `${emoji} ${kindWord} GOLD @ ${f(buy ? hi : lo)}/${f(buy ? lo : hi)} AREA\n` +
    tps.map((tp, i) => `🎯 TP${i + 1} ${f(tp)}`).join("\n") +
    `\n⛔️ SL ${f(sl)}` +
    (tag ? `\n${tag}` : "");

  return {
    channelId: src.id,
    channelName: src.name,
    topicName: useForum ? "XAUUSD Scalps" : undefined,
    text,
  };
}

/** Sygnal w formacie RM (TP w pipsach, procentowy scale-out). */
export function makeRmMessage(price: number): FeedMessage {
  const buy = Math.random() < 0.5;
  const a = price + (buy ? -R(1, 3) : R(1, 3));
  const b = a + (buy ? -R(2, 4) : R(2, 4));
  const f = (v: number) => v.toFixed(2);
  const text =
    `GOLD ${buy ? "BUY" : "SELL"} ${f(a)} — ${f(b)}\n` +
    `TP1 ${Math.round(R(20, 35))} PIPS (close 50%)\n` +
    `TP2 ${Math.round(R(50, 80))} PIPS (close 30%)\n` +
    `SL ${Math.round(R(60, 100))} PIPS`;
  return { channelId: RM.id, channelName: RM.name, text };
}

/** Komunikat zarzadzajacy do istniejacego koszyka. */
export function makeManagementMessage(b: Basket, price: number): FeedMessage | null {
  const stage = b.tpStage;
  const nextTp = b.tps[stage];
  const buy = b.direction === "BUY";
  const f = (v: number) => v.toFixed(2);

  const reached = nextTp !== undefined && (buy ? price >= nextTp : price <= nextTp);
  const options: string[] = [];

  if (reached) {
    const pips = Math.round(Math.abs(price - (b.entryLow + b.entryHigh) / 2) * 10);
    options.push(`✅ TP${stage + 1} HIT +${pips} PIPS 🎉`);
    if (stage === 0) options.push(`✅ TP1 HIT +${pips} PIPS\nSL IS SET TO BE — RISK FREE NOW 🔒`);
    if (stage >= 2)
      options.push(
        `✅ TP3 HIT +${pips} PIPS\nSECURING PARTIAL PROFITS — SL IS SET TO BE AT ${f(
          (b.entryLow + b.entryHigh) / 2,
        )}\nWILL TARGET: ${b.tps.map(f).join(", ")}`,
      );
  } else {
    const ageMin = (Date.now() - b.createdAt) / 60000;
    if (ageMin > 8 && Math.random() < 0.4) options.push("OUT AT ENTRY ON THE REST — TRADE NOT MOVING");
    // RISK FREE realnie przychodzi PO trafieniu pierwszego celu ("SL IS SET TO BE"),
    // nie na stratnym koszyku — inaczej komunikat nie miałby sensu.
    if (stage >= 1 && Math.random() < 0.3) options.push(`RISK FREE AT ${f((b.entryLow + b.entryHigh) / 2)} 🔒`);
    if (Math.random() < 0.18)
      options.push(`USE ${f(b.tps[b.tps.length - 1] + (buy ? 4 : -4))} AS TP${b.tps.length}`);
    if (Math.random() < 0.12) options.push("CANCEL THE REMAINING LIMITS ❌");
  }

  if (!options.length) return null;

  const src = CHANNELS.find((c) => c.name === b.source) ?? ATFX;
  return {
    channelId: src.id,
    channelName: src.name,
    text: pick(options),
    replyToBasket: b.id,
  };
}

export function makeChatterMessage(): FeedMessage {
  const src = Math.random() < 0.7 ? ATFX : FORUM;
  return {
    channelId: src.id,
    channelName: src.name,
    topicName: src.isForum ? "Market Talk" : undefined,
    text: pick(CHATTER),
  };
}

/** Wiadomosci historyczne — czat nie jest pusty przy pierwszym wejsciu. */
export function seedHistory(price: number): { text: string; channelId: number; channelName: string; age: number }[] {
  const f = (v: number) => v.toFixed(2);
  const base = price;
  return [
    {
      age: 1000 * 60 * 214,
      channelId: ATFX.id,
      channelName: ATFX.name,
      text: "SYNTHETIC CHATTER: WAITING FOR THE FIRST VALID SETUP.",
    },
    {
      age: 1000 * 60 * 188,
      channelId: ATFX.id,
      channelName: ATFX.name,
      text: `🟢 BUY LIMITS GOLD @ ${f(base - 12)}/${f(base - 19)} AREA\n🎯 TP1 ${f(base - 7)}\n🎯 TP2 ${f(
        base - 2,
      )}\n🎯 TP3 ${f(base + 4)}\n⛔️ SL ${f(base - 31)}\nHIGH RISK TRADE`,
    },
    {
      age: 1000 * 60 * 151,
      channelId: ATFX.id,
      channelName: ATFX.name,
      text: "TP1 HIT +48 PIPS\nRISK FREE NOW",
    },
    {
      age: 1000 * 60 * 132,
      channelId: ATFX.id,
      channelName: ATFX.name,
      text: "TP2 HIT +97 PIPS",
    },
    {
      age: 1000 * 60 * 96,
      channelId: FORUM.id,
      channelName: FORUM.name,
      text: `🔴 SELL LIMITS GOLD @ ${f(base + 16)}/${f(base + 22)} AREA\n🎯 TP1 ${f(base + 11)}\n🎯 TP2 ${f(
        base + 6,
      )}\n⛔️ SL ${f(base + 34)}`,
    },
    {
      age: 1000 * 60 * 64,
      channelId: FORUM.id,
      channelName: FORUM.name,
      text: "OUT AT ENTRY ON THE REST — MARKET LOST MOMENTUM",
    },
    {
      age: 1000 * 60 * 41,
      channelId: RM.id,
      channelName: RM.name,
      text: `GOLD BUY ${f(base - 4)} — ${f(base - 7)}\nTP1 30 PIPS (close 50%)\nTP2 65 PIPS (close 30%)\nSL 80 PIPS`,
    },
    {
      age: 1000 * 60 * 17,
      channelId: ATFX.id,
      channelName: ATFX.name,
      text: "SYNTHETIC CHATTER: OBSERVING PRICE ACTION.",
    },
  ];
}
