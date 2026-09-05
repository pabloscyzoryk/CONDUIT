//! Kontrola odczytu TWP GOLD po dopisaniu slownika celow i stopu.
use conduit_core::parser::parse;
fn main() {
    let t = "XAU/USD | Potential Upward Movement\nBuy Market Order @ 4269.448\nTarget Profit 1 @ 4277.436\nTarget Profit 2 @ 4301.611\nStop Loss @ 4253.471\nThis trading signal is not financial advice.";
    println!("TWP  → {:?}", parse(t));
    println!(
        "ZEN  → {:?}",
        parse("XAUUSD Buy 4413-4407\nTP1 4416\nSL 4400")
    );
    println!(
        "SYN  → {:?}",
        parse("BUY LIMITS GOLD @ 4100/4094\nTP 4103\nTP 4107\nTP OPEN\nSL 4093")
    );
}
