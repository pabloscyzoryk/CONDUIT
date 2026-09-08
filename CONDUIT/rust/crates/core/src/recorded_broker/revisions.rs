//! Process-local source revision tokens are observed inputs, not strategy math.
//! The live allocator is unchanged; replay supplies its recorded outputs only
//! in the verifier thread. Tokens consumed by other threads do not interfere.
use std::cell::RefCell;
enum State {
    Record(Vec<u64>),
    Replay {
        tokens: Vec<u64>,
        next: usize,
        missing: bool,
    },
}
thread_local! {static STATE:RefCell<Option<State>>=const{RefCell::new(None)};}
pub fn token(live: impl FnOnce() -> u64) -> u64 {
    STATE.with(|state| match &mut *state.borrow_mut() {
        None => live(),
        Some(State::Record(v)) => {
            let value = live();
            v.push(value);
            value
        }
        Some(State::Replay {
            tokens,
            next,
            missing,
        }) => {
            let value = tokens.get(*next).copied();
            *next += 1;
            match value {
                Some(v) => v,
                None => {
                    *missing = true;
                    0
                }
            }
        }
    })
}
pub struct Scope {
    nested: bool,
    _thread: std::marker::PhantomData<*mut ()>,
}
impl Scope {
    fn begin(state: State) -> Self {
        let nested = STATE.with(|s| {
            let mut s = s.borrow_mut();
            if s.is_some() {
                true
            } else {
                *s = Some(state);
                false
            }
        });
        Self {
            nested,
            _thread: std::marker::PhantomData,
        }
    }
    pub fn record() -> Self {
        Self::begin(State::Record(vec![]))
    }
    pub fn replay(tokens: Vec<u64>) -> Self {
        Self::begin(State::Replay {
            tokens,
            next: 0,
            missing: false,
        })
    }
    pub fn finish(self) -> Result<Vec<u64>, String> {
        if self.nested {
            return Err("nested source token scope".into());
        }
        STATE.with(|s| match s.borrow_mut().take() {
            Some(State::Record(tokens)) => Ok(tokens),
            Some(State::Replay {
                tokens,
                next,
                missing,
            }) if !missing && next == tokens.len() => Ok(tokens),
            _ => Err("source revision token transcript mismatch".into()),
        })
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if !self.nested {
            STATE.with(|s| {
                s.borrow_mut().take();
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocator_output_not_normalized_and_replay_never_calls_live() {
        let s = Scope::record();
        assert_eq!(token(|| 37), 37);
        assert_eq!(token(|| 103), 103);
        let saved = s.finish().unwrap();
        let s = Scope::replay(saved);
        assert_eq!(token(|| panic!("offline allocator must not run")), 37);
        assert_eq!(token(|| panic!("offline allocator must not run")), 103);
        assert!(s.finish().is_ok());
    }
    #[test]
    fn missing_and_extra_tokens_are_errors() {
        let s = Scope::replay(vec![]);
        token(|| 99);
        assert!(s.finish().is_err());
        let s = Scope::replay(vec![1]);
        assert!(s.finish().is_err());
    }
}
