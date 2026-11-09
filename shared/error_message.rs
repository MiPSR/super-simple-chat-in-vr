//! Creator diagnostics are untrusted output, not an unlimited log/IPC payload.
use std::fmt::{self,Display,Write};
const MAX_BYTES: usize = 2048;
struct Limited { text: String, truncated: bool }
impl Write for Limited {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        for character in value.chars() {
            let character=if character.is_control() && !matches!(character,'\n'|'\t') {'\u{fffd}'} else {character};
            if self.text.len()+character.len_utf8()>MAX_BYTES-3 {
                self.truncated=true;
                return Err(fmt::Error);
            }
            self.text.push(character);
        }
        Ok(())
    }
}
pub fn bounded(error: impl Display) -> String {
    let mut output=Limited {text:String::with_capacity(MAX_BYTES),truncated:false};
    let _=write!(&mut output,"{error}");
    if output.truncated {output.text.push('…');}
    output.text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_stop_formatting_and_remain_bounded_valid_utf8() {
        let error=bounded("🙂".repeat(4096));
        assert!(error.len()<=MAX_BYTES && error.ends_with('…'));
        assert!(!bounded("\x1b[31m\rhidden\x08").contains('\x1b'));
        struct Long<'a>(&'a std::cell::Cell<usize>);
        impl Display for Long<'_> {
            fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
                for _ in 0..1_000_000 {self.0.set(self.0.get()+1);output.write_str("x")?;}
                Ok(())
            }
        }
        let calls=std::cell::Cell::new(0);
        assert!(bounded(Long(&calls)).len()<=MAX_BYTES);
        assert!(calls.get()<MAX_BYTES+1);
    }
}
