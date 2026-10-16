use std::collections::HashMap;
use std::fmt::Display;
use std::str::FromStr;

mod keymaps {
    pub static US: [(u8, [char; 2]); 53] = [
        (orbclient::K_ESC, ['\0', '\0']),
        (orbclient::K_1, ['1', '!']),
        (orbclient::K_2, ['2', '@']),
        (orbclient::K_3, ['3', '#']),
        (orbclient::K_4, ['4', '$']),
        (orbclient::K_5, ['5', '%']),
        (orbclient::K_6, ['6', '^']),
        (orbclient::K_7, ['7', '&']),
        (orbclient::K_8, ['8', '*']),
        (orbclient::K_9, ['9', '(']),
        (orbclient::K_0, ['0', ')']),
        (orbclient::K_MINUS, ['-', '_']),
        (orbclient::K_EQUALS, ['=', '+']),
        (orbclient::K_BKSP, ['\0', '\0']),
        (orbclient::K_TAB, ['\t', '\t']),
        (orbclient::K_Q, ['q', 'Q']),
        (orbclient::K_W, ['w', 'W']),
        (orbclient::K_E, ['e', 'E']),
        (orbclient::K_R, ['r', 'R']),
        (orbclient::K_T, ['t', 'T']),
        (orbclient::K_Y, ['y', 'Y']),
        (orbclient::K_U, ['u', 'U']),
        (orbclient::K_I, ['i', 'I']),
        (orbclient::K_O, ['o', 'O']),
        (orbclient::K_P, ['p', 'P']),
        (orbclient::K_BRACE_OPEN, ['[', '{']),
        (orbclient::K_BRACE_CLOSE, [']', '}']),
        (orbclient::K_ENTER, ['\0', '\0']),
        (orbclient::K_CTRL, ['\0', '\0']),
        (orbclient::K_A, ['a', 'A']),
        (orbclient::K_S, ['s', 'S']),
        (orbclient::K_D, ['d', 'D']),
        (orbclient::K_F, ['f', 'F']),
        (orbclient::K_G, ['g', 'G']),
        (orbclient::K_H, ['h', 'H']),
        (orbclient::K_J, ['j', 'J']),
        (orbclient::K_K, ['k', 'K']),
        (orbclient::K_L, ['l', 'L']),
        (orbclient::K_SEMICOLON, [';', ':']),
        (orbclient::K_QUOTE, ['\'', '"']),
        (orbclient::K_TICK, ['`', '~']),
        (orbclient::K_BACKSLASH, ['\\', '|']),
        (orbclient::K_Z, ['z', 'Z']),
        (orbclient::K_X, ['x', 'X']),
        (orbclient::K_C, ['c', 'C']),
        (orbclient::K_V, ['v', 'V']),
        (orbclient::K_B, ['b', 'B']),
        (orbclient::K_N, ['n', 'N']),
        (orbclient::K_M, ['m', 'M']),
        (orbclient::K_COMMA, [',', '<']),
        (orbclient::K_PERIOD, ['.', '>']),
        (orbclient::K_SLASH, ['/', '?']),
        (orbclient::K_SPACE, [' ', ' ']),
    ];

    pub static GB: [(u8, [char; 2]); 54] = [
        (orbclient::K_ESC, ['\0', '\0']),
        (orbclient::K_1, ['1', '!']),
        (orbclient::K_2, ['2', '"']),
        (orbclient::K_3, ['3', '£']),
        (orbclient::K_4, ['4', '$']),
        (orbclient::K_5, ['5', '%']),
        (orbclient::K_6, ['6', '^']),
        (orbclient::K_7, ['7', '&']),
        (orbclient::K_8, ['8', '*']),
        (orbclient::K_9, ['9', '(']),
        (orbclient::K_0, ['0', ')']),
        (orbclient::K_MINUS, ['-', '_']),
        (orbclient::K_EQUALS, ['=', '+']),
        (orbclient::K_BKSP, ['\0', '\0']),
        (orbclient::K_TAB, ['\t', '\t']),
        (orbclient::K_Q, ['q', 'Q']),
        (orbclient::K_W, ['w', 'W']),
        (orbclient::K_E, ['e', 'E']),
        (orbclient::K_R, ['r', 'R']),
        (orbclient::K_T, ['t', 'T']),
        (orbclient::K_Y, ['y', 'Y']),
        (orbclient::K_U, ['u', 'U']),
        (orbclient::K_I, ['i', 'I']),
        (orbclient::K_O, ['o', 'O']),
        (orbclient::K_P, ['p', 'P']),
        (orbclient::K_BRACE_OPEN, ['[', '{']),
        (orbclient::K_BRACE_CLOSE, [']', '}']),
        (orbclient::K_ENTER, ['\0', '\0']),
        (orbclient::K_CTRL, ['\0', '\0']),
        (orbclient::K_A, ['a', 'A']),
        (orbclient::K_S, ['s', 'S']),
        (orbclient::K_D, ['d', 'D']),
        (orbclient::K_F, ['f', 'F']),
        (orbclient::K_G, ['g', 'G']),
        (orbclient::K_H, ['h', 'H']),
        (orbclient::K_J, ['j', 'J']),
        (orbclient::K_K, ['k', 'K']),
        (orbclient::K_L, ['l', 'L']),
        (orbclient::K_SEMICOLON, [';', ':']),
        (orbclient::K_QUOTE, ['\'', '@']),
        (orbclient::K_TICK, ['`', '¬']),
        (orbclient::K_BACKSLASH, ['#', '~']),
        (orbclient::K_Z, ['z', 'Z']),
        (orbclient::K_X, ['x', 'X']),
        (orbclient::K_C, ['c', 'C']),
        (orbclient::K_V, ['v', 'V']),
        (orbclient::K_B, ['b', 'B']),
        (orbclient::K_N, ['n', 'N']),
        (orbclient::K_M, ['m', 'M']),
        (orbclient::K_COMMA, [',', '<']),
        (orbclient::K_PERIOD, ['.', '>']),
        (orbclient::K_SLASH, ['/', '?']),
        (orbclient::K_SPACE, [' ', ' ']),
        // UK Backslash, doesn't exist on US keyboard
        (0x56, ['\\', '|']),
    ];

    pub static DVORAK: [(u8, [char; 2]); 53] = [
        (orbclient::K_ESC, ['\0', '\0']),
        (orbclient::K_1, ['1', '!']),
        (orbclient::K_2, ['2', '@']),
        (orbclient::K_3, ['3', '#']),
        (orbclient::K_4, ['4', '$']),
        (orbclient::K_5, ['5', '%']),
        (orbclient::K_6, ['6', '^']),
        (orbclient::K_7, ['7', '&']),
        (orbclient::K_8, ['8', '*']),
        (orbclient::K_9, ['9', '(']),
        (orbclient::K_0, ['0', ')']),
        (orbclient::K_MINUS, ['[', '{']),
        (orbclient::K_EQUALS, [']', '}']),
        (orbclient::K_BKSP, ['\0', '\0']),
        (orbclient::K_TAB, ['\t', '\t']),
        (orbclient::K_Q, ['\'', '"']),
        (orbclient::K_W, [',', '<']),
        (orbclient::K_E, ['.', '>']),
        (orbclient::K_R, ['p', 'P']),
        (orbclient::K_T, ['y', 'Y']),
        (orbclient::K_Y, ['f', 'F']),
        (orbclient::K_U, ['g', 'G']),
        (orbclient::K_I, ['c', 'C']),
        (orbclient::K_O, ['r', 'R']),
        (orbclient::K_P, ['l', 'L']),
        (orbclient::K_BRACE_OPEN, ['/', '?']),
        (orbclient::K_BRACE_CLOSE, ['=', '+']),
        (orbclient::K_ENTER, ['\0', '\0']),
        (orbclient::K_CTRL, ['\0', '\0']),
        (orbclient::K_A, ['a', 'A']),
        (orbclient::K_S, ['o', 'O']),
        (orbclient::K_D, ['e', 'E']),
        (orbclient::K_F, ['u', 'U']),
        (orbclient::K_G, ['i', 'I']),
        (orbclient::K_H, ['d', 'D']),
        (orbclient::K_J, ['h', 'H']),
        (orbclient::K_K, ['t', 'T']),
        (orbclient::K_L, ['n', 'N']),
        (orbclient::K_SEMICOLON, ['s', 'S']),
        (orbclient::K_QUOTE, ['-', '_']),
        (orbclient::K_TICK, ['`', '~']),
        (orbclient::K_BACKSLASH, ['\\', '|']),
        (orbclient::K_Z, [';', ':']),
        (orbclient::K_X, ['q', 'Q']),
        (orbclient::K_C, ['j', 'J']),
        (orbclient::K_V, ['k', 'K']),
        (orbclient::K_B, ['x', 'X']),
        (orbclient::K_N, ['b', 'B']),
        (orbclient::K_M, ['m', 'M']),
        (orbclient::K_COMMA, ['w', 'W']),
        (orbclient::K_PERIOD, ['v', 'V']),
        (orbclient::K_SLASH, ['z', 'Z']),
        (orbclient::K_SPACE, [' ', ' ']),
    ];

    pub static AZERTY: [(u8, [char; 2]); 53] = [
        (orbclient::K_ESC, ['\0', '\0']),
        (orbclient::K_1, ['&', '1']),
        (orbclient::K_2, ['é', '2']),
        (orbclient::K_3, ['"', '3']),
        (orbclient::K_4, ['\'', '4']),
        (orbclient::K_5, ['(', '5']),
        (orbclient::K_6, ['|', '6']),
        (orbclient::K_7, ['è', '7']),
        (orbclient::K_8, ['_', '8']),
        (orbclient::K_9, ['ç', '9']),
        (orbclient::K_0, ['à', '0']),
        (orbclient::K_MINUS, [')', '°']),
        (orbclient::K_EQUALS, ['=', '+']),
        (orbclient::K_BKSP, ['\0', '\0']),
        (orbclient::K_TAB, ['\t', '\t']),
        (orbclient::K_Q, ['a', 'A']),
        (orbclient::K_W, ['z', 'Z']),
        (orbclient::K_E, ['e', 'E']),
        (orbclient::K_R, ['r', 'R']),
        (orbclient::K_T, ['t', 'T']),
        (orbclient::K_Y, ['y', 'Y']),
        (orbclient::K_U, ['u', 'U']),
        (orbclient::K_I, ['i', 'I']),
        (orbclient::K_O, ['o', 'O']),
        (orbclient::K_P, ['p', 'P']),
        (orbclient::K_BRACE_OPEN, ['^', '¨']),
        (orbclient::K_BRACE_CLOSE, ['$', '£']),
        (orbclient::K_ENTER, ['\0', '\0']),
        (orbclient::K_CTRL, ['\0', '\0']),
        (orbclient::K_A, ['q', 'Q']),
        (orbclient::K_S, ['s', 'S']),
        (orbclient::K_D, ['d', 'D']),
        (orbclient::K_F, ['f', 'F']),
        (orbclient::K_G, ['g', 'G']),
        (orbclient::K_H, ['h', 'H']),
        (orbclient::K_J, ['j', 'J']),
        (orbclient::K_K, ['k', 'K']),
        (orbclient::K_L, ['l', 'L']),
        (orbclient::K_SEMICOLON, ['m', 'M']),
        (orbclient::K_QUOTE, ['ù', '%']),
        (orbclient::K_TICK, ['*', 'µ']),
        (orbclient::K_BACKSLASH, ['ê', 'Ê']),
        (orbclient::K_Z, ['w', 'W']),
        (orbclient::K_X, ['x', 'X']),
        (orbclient::K_C, ['c', 'C']),
        (orbclient::K_V, ['v', 'V']),
        (orbclient::K_B, ['b', 'B']),
        (orbclient::K_N, ['n', 'N']),
        (orbclient::K_M, [',', '?']),
        (orbclient::K_COMMA, [';', '.']),
        (orbclient::K_PERIOD, [':', '/']),
        (orbclient::K_SLASH, ['!', '§']),
        (orbclient::K_SPACE, [' ', ' ']),
    ];

    pub static BEPO: [(u8, [char; 2]); 53] = [
        (orbclient::K_ESC, ['\0', '\0']),
        (orbclient::K_1, ['"', '1']),
        (orbclient::K_2, ['«', '2']),
        (orbclient::K_3, ['»', '3']),
        (orbclient::K_4, ['(', '4']),
        (orbclient::K_5, [')', '5']),
        (orbclient::K_6, ['@', '6']),
        (orbclient::K_7, ['+', '7']),
        (orbclient::K_8, ['-', '8']),
        (orbclient::K_9, ['/', '9']),
        (orbclient::K_0, ['*', '0']),
        (orbclient::K_MINUS, ['=', '°']),
        (orbclient::K_EQUALS, ['%', '`']),
        (orbclient::K_BKSP, ['\0', '\0']),
        (orbclient::K_TAB, ['\t', '\t']),
        (orbclient::K_Q, ['b', 'B']),
        (orbclient::K_W, ['é', 'É']),
        (orbclient::K_E, ['p', 'P']),
        (orbclient::K_R, ['o', 'O']),
        (orbclient::K_T, ['è', 'È']),
        (orbclient::K_Y, ['^', '!']),
        (orbclient::K_U, ['v', 'V']),
        (orbclient::K_I, ['d', 'D']),
        (orbclient::K_O, ['l', 'L']),
        (orbclient::K_P, ['j', 'J']),
        (orbclient::K_BRACE_OPEN, ['z', 'Z']),
        (orbclient::K_BRACE_CLOSE, ['w', 'W']),
        (orbclient::K_ENTER, ['\0', '\0']),
        (orbclient::K_CTRL, ['\0', '\0']),
        (orbclient::K_A, ['a', 'A']),
        (orbclient::K_S, ['u', 'U']),
        (orbclient::K_D, ['i', 'I']),
        (orbclient::K_F, ['e', 'E']),
        (orbclient::K_G, [',', ';']),
        (orbclient::K_H, ['c', 'C']),
        (orbclient::K_J, ['t', 'T']),
        (orbclient::K_K, ['s', 'S']),
        (orbclient::K_L, ['r', 'R']),
        (orbclient::K_SEMICOLON, ['n', 'N']),
        (orbclient::K_QUOTE, ['m', 'M']),
        (orbclient::K_TICK, ['ç', 'Ç']),
        (orbclient::K_BACKSLASH, ['ê', 'Ê']),
        (orbclient::K_Z, ['à', 'À']),
        (orbclient::K_X, ['y', 'Y']),
        (orbclient::K_C, ['x', 'X']),
        (orbclient::K_V, ['.', ':']),
        (orbclient::K_B, ['k', 'K']),
        (orbclient::K_N, ['\'', '?']),
        (orbclient::K_M, ['q', 'Q']),
        (orbclient::K_COMMA, ['g', 'G']),
        (orbclient::K_PERIOD, ['h', 'H']),
        (orbclient::K_SLASH, ['f', 'F']),
        (orbclient::K_SPACE, [' ', ' ']),
    ];

    pub static IT: [(u8, [char; 2]); 53] = [
        (orbclient::K_ESC, ['\0', '\0']),
        (orbclient::K_1, ['1', '!']),
        (orbclient::K_2, ['2', '"']),
        (orbclient::K_3, ['3', '£']),
        (orbclient::K_4, ['4', '$']),
        (orbclient::K_5, ['5', '%']),
        (orbclient::K_6, ['6', '&']),
        (orbclient::K_7, ['7', '/']),
        (orbclient::K_8, ['8', '(']),
        (orbclient::K_9, ['9', ')']),
        (orbclient::K_0, ['0', '=']),
        (orbclient::K_MINUS, ['?', '\'']),
        (orbclient::K_EQUALS, ['ì', '^']),
        (orbclient::K_BKSP, ['\0', '\0']),
        (orbclient::K_TAB, ['\t', '\t']),
        (orbclient::K_Q, ['q', 'Q']),
        (orbclient::K_W, ['w', 'W']),
        (orbclient::K_E, ['e', 'E']),
        (orbclient::K_R, ['r', 'R']),
        (orbclient::K_T, ['t', 'T']),
        (orbclient::K_Y, ['y', 'Y']),
        (orbclient::K_U, ['u', 'U']),
        (orbclient::K_I, ['i', 'I']),
        (orbclient::K_O, ['o', 'O']),
        (orbclient::K_P, ['p', 'P']),
        (orbclient::K_BRACE_OPEN, ['è', 'é']),
        (orbclient::K_BRACE_CLOSE, ['+', '*']),
        (orbclient::K_ENTER, ['\0', '\0']),
        (orbclient::K_CTRL, ['\0', '\0']),
        (orbclient::K_A, ['a', 'A']),
        (orbclient::K_S, ['s', 'S']),
        (orbclient::K_D, ['d', 'D']),
        (orbclient::K_F, ['f', 'F']),
        (orbclient::K_G, ['g', 'G']),
        (orbclient::K_H, ['h', 'H']),
        (orbclient::K_J, ['j', 'J']),
        (orbclient::K_K, ['k', 'K']),
        (orbclient::K_L, ['l', 'L']),
        (orbclient::K_SEMICOLON, ['ò', 'ç']),
        (orbclient::K_QUOTE, ['à', '°']),
        (orbclient::K_TICK, ['ù', '§']),
        (orbclient::K_BACKSLASH, ['<', '>']),
        (orbclient::K_Z, ['z', 'Z']),
        (orbclient::K_X, ['x', 'X']),
        (orbclient::K_C, ['c', 'C']),
        (orbclient::K_V, ['v', 'V']),
        (orbclient::K_B, ['b', 'B']),
        (orbclient::K_N, ['n', 'N']),
        (orbclient::K_M, ['m', 'M']),
        (orbclient::K_COMMA, [',', ';']),
        (orbclient::K_PERIOD, ['.', ':']),
        (orbclient::K_SLASH, ['-', '_']),
        (orbclient::K_SPACE, [' ', ' ']),
    ];

    /// Brazilian ABNT2, as the standard prints it.
    ///
    /// The accents `´`, `` ` ``, `~`, `^` and `¨` are dead keys: they print
    /// nothing and wait for the next key, which decides what the two made
    /// together (see `KeymapData::type_key`).
    pub static ABNT2: [(u8, [char; 2]); 55] = [
        (orbclient::K_ESC, ['\0', '\0']),
        (orbclient::K_1, ['1', '!']),
        (orbclient::K_2, ['2', '@']),
        (orbclient::K_3, ['3', '#']),
        (orbclient::K_4, ['4', '$']),
        (orbclient::K_5, ['5', '%']),
        (orbclient::K_6, ['6', '¨']),
        (orbclient::K_7, ['7', '&']),
        (orbclient::K_8, ['8', '*']),
        (orbclient::K_9, ['9', '(']),
        (orbclient::K_0, ['0', ')']),
        (orbclient::K_MINUS, ['-', '_']),
        (orbclient::K_EQUALS, ['=', '+']),
        (orbclient::K_BKSP, ['\0', '\0']),
        (orbclient::K_TAB, ['\t', '\t']),
        (orbclient::K_Q, ['q', 'Q']),
        (orbclient::K_W, ['w', 'W']),
        (orbclient::K_E, ['e', 'E']),
        (orbclient::K_R, ['r', 'R']),
        (orbclient::K_T, ['t', 'T']),
        (orbclient::K_Y, ['y', 'Y']),
        (orbclient::K_U, ['u', 'U']),
        (orbclient::K_I, ['i', 'I']),
        (orbclient::K_O, ['o', 'O']),
        (orbclient::K_P, ['p', 'P']),
        (orbclient::K_BRACE_OPEN, ['´', '`']),
        (orbclient::K_BRACE_CLOSE, ['[', '{']),
        (orbclient::K_ENTER, ['\0', '\0']),
        (orbclient::K_CTRL, ['\0', '\0']),
        (orbclient::K_A, ['a', 'A']),
        (orbclient::K_S, ['s', 'S']),
        (orbclient::K_D, ['d', 'D']),
        (orbclient::K_F, ['f', 'F']),
        (orbclient::K_G, ['g', 'G']),
        (orbclient::K_H, ['h', 'H']),
        (orbclient::K_J, ['j', 'J']),
        (orbclient::K_K, ['k', 'K']),
        (orbclient::K_L, ['l', 'L']),
        (orbclient::K_SEMICOLON, ['ç', 'Ç']),
        (orbclient::K_QUOTE, ['~', '^']),
        (orbclient::K_TICK, ['\'', '"']),
        (orbclient::K_BACKSLASH, [']', '}']),
        (orbclient::K_Z, ['z', 'Z']),
        (orbclient::K_X, ['x', 'X']),
        (orbclient::K_C, ['c', 'C']),
        (orbclient::K_V, ['v', 'V']),
        (orbclient::K_B, ['b', 'B']),
        (orbclient::K_N, ['n', 'N']),
        (orbclient::K_M, ['m', 'M']),
        (orbclient::K_COMMA, [',', '<']),
        (orbclient::K_PERIOD, ['.', '>']),
        (orbclient::K_SLASH, [';', ':']),
        (orbclient::K_SPACE, [' ', ' ']),
        // The 102ND key, left of Z: backslash and pipe, where the US layout
        // has the `] }` key.
        (0x56, ['\\', '|']),
        // Slash and question, between the right shift and `; :`.
        (0x59, ['/', '?']),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(usize)]
pub enum KeymapKind {
    US = 0,
    GB,
    Dvorak,
    Azerty,
    Bepo,
    IT,
    Abnt2,
}

impl KeymapKind {
    /// Every layout, in the order of the numbers the control protocol uses.
    pub const ALL: [KeymapKind; 7] = [
        KeymapKind::US,
        KeymapKind::GB,
        KeymapKind::Dvorak,
        KeymapKind::Azerty,
        KeymapKind::Bepo,
        KeymapKind::IT,
        KeymapKind::Abnt2,
    ];
}

impl From<usize> for KeymapKind {
    fn from(value: usize) -> Self {
        if value > (KeymapKind::Abnt2 as usize) {
            KeymapKind::US
        } else {
            // SAFETY: Checked above
            unsafe { std::mem::transmute::<usize, KeymapKind>(value) }
        }
    }
}

#[allow(missing_copy_implementations)]
#[derive(Debug, PartialEq, Eq)]
pub struct ParseKeymapError(());

impl FromStr for KeymapKind {
    type Err = ParseKeymapError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let keymap = match s {
            "dvorak" => KeymapKind::Dvorak,
            "us" => KeymapKind::US,
            "gb" => KeymapKind::GB,
            "azerty" => KeymapKind::Azerty,
            "bepo" => KeymapKind::Bepo,
            "it" => KeymapKind::IT,
            // The name the X11 layouts use, and the shorter one.
            "br-abnt2" | "abnt2" => KeymapKind::Abnt2,
            &_ => return Err(ParseKeymapError(())),
        };

        Ok(keymap)
    }
}

impl Display for KeymapKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match *self {
            KeymapKind::US => "us",
            KeymapKind::GB => "gb",
            KeymapKind::Dvorak => "dvorak",
            KeymapKind::Azerty => "azerty",
            KeymapKind::Bepo => "bepo",
            KeymapKind::IT => "it",
            KeymapKind::Abnt2 => "br-abnt2",
        };
        f.write_str(s)
    }
}

pub struct KeymapData {
    pub keymap_hash: HashMap<u8, [char; 2]>,
    pub kind: KeymapKind,
    /// The dead key the next key is waiting on, if any.
    pending: Option<char>,
}

impl KeymapData {
    pub fn new(kind: KeymapKind) -> Self {
        let keymap_hash = match kind {
            KeymapKind::US => HashMap::from(keymaps::US),
            KeymapKind::GB => HashMap::from(keymaps::GB),
            KeymapKind::Dvorak => HashMap::from(keymaps::DVORAK),
            KeymapKind::Azerty => HashMap::from(keymaps::AZERTY),
            KeymapKind::Bepo => HashMap::from(keymaps::BEPO),
            KeymapKind::IT => HashMap::from(keymaps::IT),
            KeymapKind::Abnt2 => HashMap::from(keymaps::ABNT2),
        };

        Self {
            keymap_hash,
            kind,
            pending: None,
        }
    }

    pub fn get_kind(&self) -> KeymapKind {
        self.kind
    }

    // TODO: AltGr, Numlock
    pub fn get_char(&self, scancode: u8, shift: bool) -> char {
        if let Some(c) = self.keymap_hash.get(&scancode) {
            if shift {
                c[1]
            } else {
                c[0]
            }
        } else {
            '\0'
        }
    }

    /// Whether the layout holds a dead key in this character.
    ///
    /// Only the Brazilian layout: the accents `´`, `` ` ``, `~`, `^` and `¨`
    /// print nothing on their own and wait for the next key.
    pub fn is_dead_key(&self, c: char) -> bool {
        self.kind == KeymapKind::Abnt2 && matches!(c, '´' | '`' | '~' | '^' | '¨')
    }

    /// Types a key: dead keys included. Call it on presses; a release prints
    /// the layout as it is.
    ///
    /// A dead key waits for the next key and prints nothing by itself: `´`
    /// then `a` is `á`. A dead key that has nothing to compose with is printed
    /// before what was typed — `´` then `b` is `´b` — and a key that prints
    /// nothing of its own leaves the dead key behind.
    pub fn type_key(&mut self, scancode: u8, shift: bool) -> Typed {
        let c = self.get_char(scancode, shift);

        if c == '\0' {
            // Not a character key (Enter, an arrow): it cannot compose, and it
            // leaves the waiting dead key behind.
            self.pending = None;
            return Typed::Nothing;
        }

        if self.is_dead_key(c) {
            return match self.pending.replace(c) {
                // A dead key that waited for a composer got none: it is
                // printed, and the new one waits.
                Some(pending) => Typed::Char(pending),
                None => Typed::Nothing,
            };
        }

        if let Some(pending) = self.pending.take() {
            if let Some(composed) = compose(pending, c) {
                return Typed::Char(composed);
            }
            // A dead key that did not compose is printed before what was
            // typed.
            return Typed::Chars(pending, c);
        }

        Typed::Char(c)
    }
}

/// What a key printed, in the order it printed it.
#[derive(Debug, PartialEq, Eq)]
pub enum Typed {
    /// Nothing: a dead key is waiting for the next one.
    Nothing,
    /// The character of the key.
    Char(char),
    /// A dead key that did not compose, then what was typed.
    Chars(char, char),
}

/// The character a dead key and a key make together.
///
/// The accents of the dead keys and the letters they mark: `´` with `a` is
/// `á`, `~` with `o` is `õ`. Keys that make nothing together are not here.
fn compose(dead: char, key: char) -> Option<char> {
    let letter = key.to_ascii_lowercase();
    let composed = match dead {
        '´' => match letter {
            'a' => 'á',
            'e' => 'é',
            'i' => 'í',
            'o' => 'ó',
            'u' => 'ú',
            'y' => 'ý',
            _ => return None,
        },
        '`' => match letter {
            'a' => 'à',
            'e' => 'è',
            'i' => 'ì',
            'o' => 'ò',
            'u' => 'ù',
            _ => return None,
        },
        '~' => match letter {
            'a' => 'ã',
            'n' => 'ñ',
            'o' => 'õ',
            _ => return None,
        },
        '^' => match letter {
            'a' => 'â',
            'e' => 'ê',
            'i' => 'î',
            'o' => 'ô',
            'u' => 'û',
            _ => return None,
        },
        '¨' => match letter {
            'a' => 'ä',
            'e' => 'ë',
            'i' => 'ï',
            'o' => 'ö',
            'u' => 'ü',
            'y' => 'ÿ',
            _ => return None,
        },
        _ => return None,
    };

    Some(if key.is_uppercase() {
        composed.to_uppercase().next().unwrap_or(composed)
    } else {
        composed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types keys in order and returns what they printed.
    fn type_keys(kind: KeymapKind, keys: &[(u8, bool)]) -> String {
        let mut keymap = KeymapData::new(kind);
        let mut typed = String::new();

        for &(scancode, shift) in keys {
            match keymap.type_key(scancode, shift) {
                Typed::Nothing => {}
                Typed::Char(c) => typed.push(c),
                Typed::Chars(first, second) => {
                    typed.push(first);
                    typed.push(second);
                }
            }
        }

        typed
    }

    /// Types a dead key and a key after it, both of the Brazilian layout.
    fn compose_keys(dead: u8, shift_dead: bool, key: u8, shift_key: bool) -> String {
        type_keys(KeymapKind::Abnt2, &[(dead, shift_dead), (key, shift_key)])
    }

    #[test]
    fn br_abnt2_prints_the_keys_a_brazilian_keyboard_has() {
        let keymap = KeymapData::new(KeymapKind::Abnt2);

        // The keys the US layout does not have, and where the two differ.
        assert_eq!(keymap.get_char(orbclient::K_SEMICOLON, false), 'ç');
        assert_eq!(keymap.get_char(orbclient::K_SEMICOLON, true), 'Ç');
        assert_eq!(keymap.get_char(orbclient::K_QUOTE, false), '~');
        assert_eq!(keymap.get_char(orbclient::K_QUOTE, true), '^');
        assert_eq!(keymap.get_char(orbclient::K_TICK, false), '\'');
        assert_eq!(keymap.get_char(orbclient::K_TICK, true), '"');
        assert_eq!(keymap.get_char(orbclient::K_BRACE_OPEN, false), '´');
        assert_eq!(keymap.get_char(orbclient::K_BRACE_OPEN, true), '`');
        assert_eq!(keymap.get_char(orbclient::K_BRACE_CLOSE, false), '[');
        assert_eq!(keymap.get_char(orbclient::K_BRACE_CLOSE, true), '{');
        assert_eq!(keymap.get_char(orbclient::K_BACKSLASH, false), ']');
        assert_eq!(keymap.get_char(orbclient::K_BACKSLASH, true), '}');
        assert_eq!(keymap.get_char(orbclient::K_SLASH, false), ';');
        assert_eq!(keymap.get_char(orbclient::K_SLASH, true), ':');
        // The 102ND key and the slash/question key, which have no orbclient
        // constant.
        assert_eq!(keymap.get_char(0x56, false), '\\');
        assert_eq!(keymap.get_char(0x56, true), '|');
        assert_eq!(keymap.get_char(0x59, false), '/');
        assert_eq!(keymap.get_char(0x59, true), '?');
        // The six has the diaeresis, not the circumflex.
        assert_eq!(keymap.get_char(orbclient::K_6, false), '6');
        assert_eq!(keymap.get_char(orbclient::K_6, true), '¨');
        assert_eq!(keymap.get_char(orbclient::K_2, true), '@');
    }

    #[test]
    fn the_dead_keys_compose_with_the_letters() {
        // `´` and `` ` `` are on the key of `[`, `~` and `^` on the key of `'`,
        // and `¨` on the shifted six.
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, false, orbclient::K_A, false),
            "á"
        );
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, false, orbclient::K_E, false),
            "é"
        );
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, false, orbclient::K_I, false),
            "í"
        );
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, false, orbclient::K_O, false),
            "ó"
        );
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, false, orbclient::K_U, false),
            "ú"
        );
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, true, orbclient::K_A, false),
            "à"
        );
        assert_eq!(
            compose_keys(orbclient::K_QUOTE, false, orbclient::K_A, false),
            "ã"
        );
        assert_eq!(
            compose_keys(orbclient::K_QUOTE, false, orbclient::K_O, false),
            "õ"
        );
        assert_eq!(
            compose_keys(orbclient::K_QUOTE, true, orbclient::K_E, false),
            "ê"
        );
        assert_eq!(
            compose_keys(orbclient::K_6, true, orbclient::K_O, false),
            "ö"
        );
        assert_eq!(
            compose_keys(orbclient::K_6, true, orbclient::K_U, false),
            "ü"
        );
        // Shifted keys compose into the upper case: `´` and `A` are `Á`.
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, false, orbclient::K_A, true),
            "Á"
        );
        assert_eq!(
            compose_keys(orbclient::K_QUOTE, false, orbclient::K_O, true),
            "Õ"
        );
        assert_eq!(
            compose_keys(orbclient::K_QUOTE, true, orbclient::K_E, true),
            "Ê"
        );
        // `ç` is a key of its own and composes with nothing.
        assert_eq!(
            compose_keys(
                orbclient::K_BRACE_OPEN,
                false,
                orbclient::K_SEMICOLON,
                false
            ),
            "´ç"
        );
    }

    #[test]
    fn a_dead_key_that_does_not_compose_is_printed_first() {
        // There is no `b` with an acute, so both are typed.
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, false, orbclient::K_B, false),
            "´b"
        );
        // A backtick is written with the dead grave and a space.
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, true, orbclient::K_SPACE, false),
            "` "
        );
        // And it is a key of the shell, before a command.
        assert_eq!(
            compose_keys(orbclient::K_BRACE_OPEN, true, orbclient::K_L, false),
            "`l"
        );
    }

    #[test]
    fn a_dead_key_waits_for_the_next_key() {
        let mut keymap = KeymapData::new(KeymapKind::Abnt2);

        // A dead key alone prints nothing.
        assert_eq!(
            keymap.type_key(orbclient::K_BRACE_OPEN, false),
            Typed::Nothing
        );
        // A second dead key prints the first and waits itself.
        assert_eq!(keymap.type_key(orbclient::K_QUOTE, true), Typed::Char('´'));
        assert_eq!(keymap.type_key(orbclient::K_E, false), Typed::Char('ê'));
        // A key that prints nothing of its own leaves the dead key behind.
        assert_eq!(
            keymap.type_key(orbclient::K_BRACE_OPEN, false),
            Typed::Nothing
        );
        assert_eq!(keymap.type_key(orbclient::K_ENTER, false), Typed::Nothing);
        assert_eq!(keymap.type_key(orbclient::K_A, false), Typed::Char('a'));
    }

    #[test]
    fn only_the_brazilian_layout_has_dead_keys() {
        // `` ` `` and `~` print themselves in `us`, and nothing waits after
        // them.
        assert_eq!(
            type_keys(
                KeymapKind::US,
                &[(orbclient::K_TICK, false), (orbclient::K_A, false)]
            ),
            "`a"
        );
        assert_eq!(
            type_keys(
                KeymapKind::US,
                &[(orbclient::K_TICK, true), (orbclient::K_A, false)]
            ),
            "~a"
        );
        // The quote key prints itself in the Brazilian layout as well.
        assert_eq!(
            type_keys(
                KeymapKind::Abnt2,
                &[(orbclient::K_TICK, false), (orbclient::K_A, false)]
            ),
            "'a"
        );
    }

    #[test]
    fn the_layouts_are_named_and_numbered() {
        assert_eq!("br-abnt2".parse::<KeymapKind>(), Ok(KeymapKind::Abnt2));
        assert_eq!("abnt2".parse::<KeymapKind>(), Ok(KeymapKind::Abnt2));
        assert_eq!(KeymapKind::Abnt2.to_string(), "br-abnt2");
        assert_ne!(
            "br".parse::<KeymapKind>(),
            Ok(KeymapKind::Abnt2),
            "the layouts are named as they are, not as they look"
        );
        assert_eq!(KeymapKind::ALL.len(), 7);
        assert_eq!(KeymapKind::from(KeymapKind::ALL.len()), KeymapKind::US);

        for (number, kind) in KeymapKind::ALL.iter().enumerate() {
            assert_eq!(KeymapKind::from(number), *kind);
            assert_eq!(kind.to_string().parse::<KeymapKind>(), Ok(*kind));
        }
    }
}
