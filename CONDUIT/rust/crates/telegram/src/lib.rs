
pub mod client;
pub mod dialogs;
pub mod incoming;
pub mod history_fetch;
pub mod history_import;
pub mod history_adapter;
pub mod login;
pub mod luki;
pub mod photos;
pub mod qr;
pub mod service;
pub mod session;

pub use client::{
    ClientConfig, Kasowanie, PolitykaKasowania, TelegramClient, Zdarzenie, BUFOR_AKTUALIZACJI,
};
pub use dialogs::{DialogEntry, TopicEntry};
pub use incoming::{Extracted, Skasowane, GENERAL_TOPIC};
pub use login::{LoginStage, QrLogin};
pub use luki::{LicznikLuk, Luka, LukiMigawka, Skrzynka};
pub use photos::PhotoCache;
pub use qr::{AsciiStyle, QrRender};
pub use service::{
    KasowanieSink, MessageSink, TelegramService, Zdrowie, ZdrowieMigawka, PLIK_SESJI,
};
pub use session::{FileSession, SessionError};

/// Re-eksport, żeby aplikacja nie musiała sama zależeć od `grammers`.
pub use grammers_client;
