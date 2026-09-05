import type { AiModel, TelegramChannel } from "@/types";

/** Obvious synthetic data used only when the panel runs without a backend. */
export const CHANNELS: TelegramChannel[] = [
  {
    id: -1000000000001,
    name: "EXAMPLE SIGNAL CHANNEL",
    handle: "@example_signal_channel",
    members: 0,
    avatarHue: 210,
    isForum: false,
    verified: false,
    topics: [],
    lastMessage: "BUY LIMITS GOLD @ 2100/2095 AREA · TP 2105/2110 · SL 2090",
    lastMessageTime: Date.now() - 60_000,
  },
  {
    id: -1000000000002,
    name: "EXAMPLE SECONDARY CHANNEL",
    handle: "@example_secondary_channel",
    members: 0,
    avatarHue: 30,
    isForum: false,
    verified: false,
    topics: [],
    lastMessage: "GOLD BUY 2100—2095 · TP1 30 PIPS · SL 80 PIPS",
    lastMessageTime: Date.now() - 90_000,
  },
  {
    id: -1000000000003,
    name: "EXAMPLE FORUM",
    handle: "@example_signal_forum",
    members: 0,
    avatarHue: 120,
    isForum: true,
    verified: false,
    topics: [
      { id: 1, name: "Example topic A", icon: "A" },
      { id: 2, name: "Example topic B", icon: "B" },
    ],
    lastMessage: "[Example topic A] TP1 HIT",
    lastMessageTime: Date.now() - 120_000,
  },
];

/** No trained model or private model metadata ships in the public source tree. */
export const AI_MODELS: AiModel[] = [];
