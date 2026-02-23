/// Bytecode-compiled regex engine with backtracking VM.
/// Supports the Oniguruma subset needed for TextMate grammars.
/// Single-line matching only, UTF-8 aware.

// ── Public types ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Regex {
    program: Vec<Inst>,
    n_groups: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct Captures {
    pub groups: Vec<Option<Match>>,
}

// ── Bytecode ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CharClass {
    ranges: Vec<(char, char)>,
}

impl CharClass {
    fn contains(&self, ch: char) -> bool {
        self.ranges.iter().any(|&(lo, hi)| ch >= lo && ch <= hi)
    }
}

#[derive(Debug, Clone)]
enum Inst {
    Literal(char),
    AnyChar,
    CharClass(CharClass),
    NegCharClass(CharClass),
    Split(usize, usize),
    Jump(usize),
    Match,
    SaveStart(usize),
    SaveEnd(usize),
    AssertStart,
    AssertEnd,
    AssertWordBoundary,
    AssertNotWordBoundary,
    Backref(usize),
    /// Lookahead(negative, skip_to). VM tries body at pc+1; on Match XOR neg => jump to skip_to.
    Lookahead(bool, usize),
}

// ── Compiler ──────────────────────────────────────────────────
//
// Direct emission into program Vec with absolute addresses.
// Forward references use placeholder 0 and are patched after.

struct Compiler<'a> {
    pattern: &'a [u8],
    pos: usize,
    group_count: usize,
    program: Vec<Inst>,
}

impl<'a> Compiler<'a> {
    fn new(pattern: &'a str) -> Self {
        Self {
            pattern: pattern.as_bytes(),
            pos: 0,
            group_count: 0,
            program: Vec::new(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.pattern.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.pattern.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    fn emit(&mut self, inst: Inst) -> usize {
        let pc = self.program.len();
        self.program.push(inst);
        pc
    }

    fn compile(mut self) -> Result<Regex, String> {
        self.compile_alternation()?;
        if self.pos < self.pattern.len() {
            return Err(format!("Regex error at {}: unexpected ')'", self.pos));
        }
        self.emit(Inst::Match);
        Ok(Regex {
            n_groups: self.group_count + 1,
            program: self.program,
        })
    }

    fn compile_alternation(&mut self) -> Result<(), String> {
        // Two-pass approach:
        // Pass 1: parse all alternatives, recording pattern byte ranges (without emitting)
        // Pass 2: emit with proper Split/Jump structure

        // Record start of first alternative in pattern
        let pat_starts = self.scan_alternation_ranges()?;

        if pat_starts.len() == 1 {
            // Only one alternative, just compile it directly
            self.compile_sequence()?;
            return Ok(());
        }

        // Multiple alternatives: emit Split/Jump chain
        let n = pat_starts.len();
        let mut jump_fixups = Vec::new();

        for (i, &pat_start) in pat_starts.iter().enumerate() {
            if i < n - 1 {
                let split_pc = self.emit(Inst::Split(0, 0));
                let alt_start = self.program.len();
                self.pos = pat_start;
                self.compile_sequence()?;
                let jump_pc = self.emit(Inst::Jump(0));
                jump_fixups.push(jump_pc);
                let next = self.program.len();
                self.program[split_pc] = Inst::Split(alt_start, next);
            } else {
                self.pos = pat_start;
                self.compile_sequence()?;
            }
        }

        let end = self.program.len();
        for jpc in jump_fixups {
            self.program[jpc] = Inst::Jump(end);
        }

        Ok(())
    }

    /// Scan pattern to find the byte positions of each alternative, then restore pos.
    /// Returns the starting pattern positions of each alternative.
    fn scan_alternation_ranges(&mut self) -> Result<Vec<usize>, String> {
        let save_pos = self.pos;
        let mut starts = vec![self.pos];
        let mut depth = 0usize;

        while let Some(b) = self.peek() {
            match b {
                b'(' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    self.pos += 1;
                }
                b'|' if depth == 0 => {
                    self.pos += 1; // skip '|'
                    starts.push(self.pos);
                }
                b'\\' => {
                    self.pos += 1; // skip '\'
                    if self.pos < self.pattern.len() {
                        self.pos += 1; // skip escaped char
                    }
                }
                b'[' => {
                    // Skip character class
                    self.pos += 1;
                    if self.peek() == Some(b'^') {
                        self.pos += 1;
                    }
                    let mut first = true;
                    while (self.peek() != Some(b']') || first) && self.peek().is_some() {
                        first = false;
                        if self.peek() == Some(b'\\') {
                            self.pos += 1;
                        }
                        self.pos += 1;
                    }
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                    }
                }
                _ => {
                    self.pos += 1;
                }
            }
        }

        // If only one alternative, restore position and return
        if starts.len() == 1 {
            self.pos = save_pos;
            return Ok(starts);
        }

        // Save the end position (after last alternative, before ')' or EOF)
        // Don't restore pos — the caller will set pos for each alternative
        // Actually, we need the end position recorded so the caller knows where
        // the alternation ends. But the caller's compile_sequence will stop at
        // '|' or ')'. We just need to not restore pos.
        // Wait, the caller will set self.pos = pat_starts[i] for each alt.
        // After the last compile_sequence, self.pos will be at the ')' or EOF.
        // That's fine. But we need to NOT advance self.pos past that.
        // Let's just store the final position and not restore.
        // Actually, the loop above stopped at ')' or EOF, which is correct.
        // self.pos is now past all the alternatives.

        // Store end pos to restore after all alternatives compiled
        // No — the caller handles pos. Just return starts.
        // But we need to reset pos to save_pos so the caller can manage it.
        self.pos = save_pos;

        Ok(starts)
    }

    fn compile_sequence(&mut self) -> Result<(), String> {
        while let Some(b) = self.peek() {
            match b {
                b')' | b'|' => break,
                _ => self.compile_quantifier()?,
            }
        }
        Ok(())
    }

    fn compile_quantifier(&mut self) -> Result<(), String> {
        let atom_start = self.program.len();
        self.compile_atom()?;
        let atom_end = self.program.len();

        match self.peek() {
            Some(b'*') => {
                self.pos += 1;
                // We need: Split(atom, skip) atom Jump(split)
                // But atom is already emitted at atom_start..atom_end.
                // Insert Split before atom, add Jump after.
                // Inserting shifts all addresses in atom by +1.
                self.insert_before(atom_start, Inst::Split(0, 0));
                // atom is now at atom_start+1..atom_end+1
                let jump_pc = self.emit(Inst::Jump(atom_start));
                let after = self.program.len();
                self.program[atom_start] = Inst::Split(atom_start + 1, after);
                let _ = jump_pc;
            }
            Some(b'+') => {
                self.pos += 1;
                // atom Split(atom, after)
                let split_pc = self.emit(Inst::Split(atom_start, 0));
                let after = self.program.len();
                self.program[split_pc] = Inst::Split(atom_start, after);
            }
            Some(b'?') => {
                self.pos += 1;
                // Split(atom, after) atom
                self.insert_before(atom_start, Inst::Split(0, 0));
                // atom now at atom_start+1..atom_end+1
                let after = self.program.len();
                self.program[atom_start] = Inst::Split(atom_start + 1, after);
            }
            Some(b'{') => {
                self.compile_counted(atom_start, atom_end)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Insert an instruction before `pos`, shifting all addresses >= pos by +1.
    fn insert_before(&mut self, pos: usize, inst: Inst) {
        self.program.insert(pos, inst);
        // Fix all addresses that reference positions >= pos
        for i in 0..self.program.len() {
            match &mut self.program[i] {
                Inst::Split(a, b) => {
                    if *a > pos || (*a == pos && i != pos) {
                        *a += 1;
                    }
                    if *b > pos || (*b == pos && i != pos) {
                        *b += 1;
                    }
                }
                Inst::Jump(t) => {
                    if *t > pos || (*t == pos && i != pos) {
                        *t += 1;
                    }
                }
                Inst::Lookahead(_, skip) => {
                    if *skip > pos {
                        *skip += 1;
                    }
                }
                _ => {}
            }
        }
    }

    fn compile_counted(&mut self, atom_start: usize, atom_end: usize) -> Result<(), String> {
        self.pos += 1; // '{'
        let min = self.parse_decimal()?;
        let max;
        if self.peek() == Some(b',') {
            self.pos += 1;
            if self.peek() == Some(b'}') {
                max = None;
            } else {
                max = Some(self.parse_decimal()?);
            }
        } else {
            max = Some(min);
        }
        if self.advance() != Some(b'}') {
            return Err(format!("Regex error at {}: expected '}}'", self.pos));
        }

        // Get atom bytecode
        let atom_code: Vec<Inst> = self.program[atom_start..atom_end].to_vec();
        self.program.truncate(atom_start);

        // Emit min copies
        for _ in 0..min {
            for inst in &atom_code {
                self.program.push(inst.clone());
            }
        }

        match max {
            Some(m) => {
                for _ in 0..m.saturating_sub(min) {
                    let split_pc = self.emit(Inst::Split(0, 0));
                    for inst in &atom_code {
                        self.program.push(inst.clone());
                    }
                    let after = self.program.len();
                    self.program[split_pc] = Inst::Split(split_pc + 1, after);
                }
            }
            None => {
                let split_pc = self.emit(Inst::Split(0, 0));
                for inst in &atom_code {
                    self.program.push(inst.clone());
                }
                self.emit(Inst::Jump(split_pc));
                let after = self.program.len();
                self.program[split_pc] = Inst::Split(split_pc + 1, after);
            }
        }
        Ok(())
    }

    fn parse_decimal(&mut self) -> Result<usize, String> {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(format!("Regex error at {}: expected number", self.pos));
        }
        std::str::from_utf8(&self.pattern[start..self.pos])
            .unwrap()
            .parse::<usize>()
            .map_err(|_| format!("Regex error at {}: invalid number", start))
    }

    fn compile_atom(&mut self) -> Result<(), String> {
        match self.peek() {
            Some(b'(') => self.compile_group(),
            Some(b'[') => self.compile_char_class(),
            Some(b'^') => {
                self.pos += 1;
                self.emit(Inst::AssertStart);
                Ok(())
            }
            Some(b'$') => {
                self.pos += 1;
                self.emit(Inst::AssertEnd);
                Ok(())
            }
            Some(b'.') => {
                self.pos += 1;
                self.emit(Inst::AnyChar);
                Ok(())
            }
            Some(b'\\') => {
                self.pos += 1;
                self.compile_escape()
            }
            Some(b)
                if b != b')' && b != b'|' && b != b'*' && b != b'+' && b != b'?' && b != b'{' =>
            {
                self.pos += 1;
                let ch = if b >= 0x80 {
                    self.pos -= 1;
                    let s = std::str::from_utf8(&self.pattern[self.pos..])
                        .map_err(|_| format!("Regex error at {}: invalid UTF-8", self.pos))?;
                    let c = s.chars().next().unwrap();
                    self.pos += c.len_utf8();
                    c
                } else {
                    b as char
                };
                self.emit(Inst::Literal(ch));
                Ok(())
            }
            Some(b) => Err(format!(
                "Regex error at {}: unexpected '{}'",
                self.pos, b as char
            )),
            None => Err(format!(
                "Regex error at {}: unexpected end of pattern",
                self.pos
            )),
        }
    }

    fn compile_group(&mut self) -> Result<(), String> {
        self.pos += 1; // '('
        if self.peek() == Some(b'?') {
            self.pos += 1;
            match self.peek() {
                Some(b':') => {
                    self.pos += 1;
                    self.compile_alternation()?;
                    if self.advance() != Some(b')') {
                        return Err(format!("Regex error at {}: expected ')'", self.pos));
                    }
                    Ok(())
                }
                Some(b'=') => {
                    self.pos += 1;
                    let la_pc = self.emit(Inst::Lookahead(false, 0)); // placeholder
                    self.compile_alternation()?;
                    self.emit(Inst::Match); // end of lookahead body
                    let after = self.program.len();
                    self.program[la_pc] = Inst::Lookahead(false, after);
                    if self.advance() != Some(b')') {
                        return Err(format!("Regex error at {}: expected ')'", self.pos));
                    }
                    Ok(())
                }
                Some(b'!') => {
                    self.pos += 1;
                    let la_pc = self.emit(Inst::Lookahead(true, 0)); // placeholder
                    self.compile_alternation()?;
                    self.emit(Inst::Match); // end of lookahead body
                    let after = self.program.len();
                    self.program[la_pc] = Inst::Lookahead(true, after);
                    if self.advance() != Some(b')') {
                        return Err(format!("Regex error at {}: expected ')'", self.pos));
                    }
                    Ok(())
                }
                _ => {
                    // Inline mode flags: (?i), (?s), (?m), (?i-m), etc.
                    // These are zero-length mode modifiers with no body.
                    // We parse past the flags and closing ')' and emit nothing.
                    loop {
                        match self.advance() {
                            Some(b')') => return Ok(()),
                            Some(b) if b.is_ascii_alphabetic() || b == b'-' => {} // flag char
                            _ => {
                                return Err(format!(
                                    "Regex error at {}: unsupported group modifier",
                                    self.pos
                                ))
                            }
                        }
                    }
                }
            }
        } else {
            self.group_count += 1;
            let gid = self.group_count;
            self.emit(Inst::SaveStart(gid));
            self.compile_alternation()?;
            self.emit(Inst::SaveEnd(gid));
            if self.advance() != Some(b')') {
                return Err(format!("Regex error at {}: expected ')'", self.pos));
            }
            Ok(())
        }
    }

    fn compile_escape(&mut self) -> Result<(), String> {
        let inst = match self.advance() {
            Some(b'w') => Inst::CharClass(CharClass {
                ranges: vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')],
            }),
            Some(b'W') => Inst::NegCharClass(CharClass {
                ranges: vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')],
            }),
            Some(b'd') => Inst::CharClass(CharClass {
                ranges: vec![('0', '9')],
            }),
            Some(b'D') => Inst::NegCharClass(CharClass {
                ranges: vec![('0', '9')],
            }),
            Some(b's') => Inst::CharClass(CharClass {
                ranges: vec![
                    (' ', ' '),
                    ('\t', '\t'),
                    ('\n', '\n'),
                    ('\r', '\r'),
                    ('\u{000B}', '\u{000B}'),
                    ('\u{000C}', '\u{000C}'),
                ],
            }),
            Some(b'S') => Inst::NegCharClass(CharClass {
                ranges: vec![
                    (' ', ' '),
                    ('\t', '\t'),
                    ('\n', '\n'),
                    ('\r', '\r'),
                    ('\u{000B}', '\u{000B}'),
                    ('\u{000C}', '\u{000C}'),
                ],
            }),
            Some(b'b') => Inst::AssertWordBoundary,
            Some(b'B') => Inst::AssertNotWordBoundary,
            Some(b'n') => Inst::Literal('\n'),
            Some(b't') => Inst::Literal('\t'),
            Some(b'r') => Inst::Literal('\r'),
            Some(b @ b'1'..=b'9') => Inst::Backref((b - b'0') as usize),
            Some(b) if b.is_ascii_punctuation() => Inst::Literal(b as char),
            Some(b) => {
                return Err(format!(
                    "Regex error at {}: unknown escape '\\{}'",
                    self.pos - 1,
                    b as char
                ));
            }
            None => {
                return Err(format!(
                    "Regex error at {}: unexpected end after '\\'",
                    self.pos
                ));
            }
        };
        self.emit(inst);
        Ok(())
    }

    fn compile_char_class(&mut self) -> Result<(), String> {
        self.pos += 1; // '['
        let negated = if self.peek() == Some(b'^') {
            self.pos += 1;
            true
        } else {
            false
        };
        let mut ranges = Vec::new();
        let mut first = true;
        while self.peek() != Some(b']') || first {
            first = false;
            if self.peek().is_none() {
                return Err(format!(
                    "Regex error at {}: unterminated character class",
                    self.pos
                ));
            }
            let lo = self.class_atom()?;
            if self.peek() == Some(b'-') && self.pattern.get(self.pos + 1) != Some(&b']') {
                self.pos += 1;
                let hi = self.class_atom()?;
                ranges.push((lo, hi));
            } else {
                ranges.push((lo, lo));
            }
        }
        self.pos += 1; // ']'
        let cc = CharClass { ranges };
        if negated {
            self.emit(Inst::NegCharClass(cc));
        } else {
            self.emit(Inst::CharClass(cc));
        }
        Ok(())
    }

    fn class_atom(&mut self) -> Result<char, String> {
        match self.advance() {
            Some(b'\\') => match self.advance() {
                Some(b'n') => Ok('\n'),
                Some(b't') => Ok('\t'),
                Some(b'r') => Ok('\r'),
                Some(b'\\') => Ok('\\'),
                Some(b']') => Ok(']'),
                Some(b'-') => Ok('-'),
                Some(b'^') => Ok('^'),
                Some(b) => Ok(b as char),
                None => Err(format!(
                    "Regex error at {}: unexpected end in class",
                    self.pos
                )),
            },
            Some(b) if b >= 0x80 => {
                self.pos -= 1;
                let s = std::str::from_utf8(&self.pattern[self.pos..])
                    .map_err(|_| format!("Regex error at {}: invalid UTF-8", self.pos))?;
                let ch = s.chars().next().unwrap();
                self.pos += ch.len_utf8();
                Ok(ch)
            }
            Some(b) => Ok(b as char),
            None => Err(format!(
                "Regex error at {}: unexpected end in class",
                self.pos
            )),
        }
    }
}

// ── VM ────────────────────────────────────────────────────────

#[derive(Clone)]
struct VmState {
    pc: usize,
    sp: usize,
    groups: Vec<Option<usize>>,
}

impl Regex {
    pub fn new(pattern: &str) -> Result<Self, String> {
        Compiler::new(pattern).compile()
    }

    pub fn find(&self, text: &str, start: usize) -> Option<Match> {
        self.captures(text, start)?.groups[0]
    }

    pub fn captures(&self, text: &str, start: usize) -> Option<Captures> {
        let mut pos = start;
        while pos <= text.len() {
            if let Some(caps) = self.try_match(text, pos) {
                return Some(caps);
            }
            if pos < text.len() {
                pos += text[pos..].chars().next().map_or(1, |c| c.len_utf8());
            } else {
                break;
            }
        }
        None
    }

    fn try_match(&self, text: &str, start: usize) -> Option<Captures> {
        let n_slots = self.n_groups * 2;
        let mut groups = vec![None; n_slots];
        groups[0] = Some(start);

        let mut stack: Vec<VmState> = Vec::new();
        let mut pc = 0usize;
        let mut sp = start;
        let mut steps = 0usize;
        let max_steps = 100_000;
        let text_bytes = text.as_bytes();

        loop {
            steps += 1;
            if steps > max_steps {
                return None;
            }

            if pc >= self.program.len() {
                if let Some(state) = stack.pop() {
                    pc = state.pc;
                    sp = state.sp;
                    groups = state.groups;
                    continue;
                }
                return None;
            }

            match &self.program[pc] {
                Inst::Match => {
                    groups[1] = Some(sp);
                    let mut result = Vec::with_capacity(self.n_groups);
                    for g in 0..self.n_groups {
                        match (groups[g * 2], groups[g * 2 + 1]) {
                            (Some(s), Some(e)) => result.push(Some(Match { start: s, end: e })),
                            _ => result.push(None),
                        }
                    }
                    return Some(Captures { groups: result });
                }

                Inst::Literal(ch) => {
                    if sp < text.len() {
                        let actual = char_at(text, sp);
                        if actual == *ch {
                            pc += 1;
                            sp += actual.len_utf8();
                            continue;
                        }
                    }
                }

                Inst::AnyChar => {
                    if sp < text.len() {
                        let ch = char_at(text, sp);
                        if ch != '\n' {
                            pc += 1;
                            sp += ch.len_utf8();
                            continue;
                        }
                    }
                }

                Inst::CharClass(cc) => {
                    if sp < text.len() {
                        let ch = char_at(text, sp);
                        if cc.contains(ch) {
                            pc += 1;
                            sp += ch.len_utf8();
                            continue;
                        }
                    }
                }

                Inst::NegCharClass(cc) => {
                    if sp < text.len() {
                        let ch = char_at(text, sp);
                        if !cc.contains(ch) && ch != '\n' {
                            pc += 1;
                            sp += ch.len_utf8();
                            continue;
                        }
                    }
                }

                Inst::Split(a, b) => {
                    let (a, b) = (*a, *b);
                    stack.push(VmState {
                        pc: b,
                        sp,
                        groups: groups.clone(),
                    });
                    pc = a;
                    continue;
                }

                Inst::Jump(target) => {
                    pc = *target;
                    continue;
                }

                Inst::SaveStart(g) => {
                    let g = *g;
                    if g * 2 < groups.len() {
                        groups[g * 2] = Some(sp);
                    }
                    pc += 1;
                    continue;
                }

                Inst::SaveEnd(g) => {
                    let g = *g;
                    if g * 2 + 1 < groups.len() {
                        groups[g * 2 + 1] = Some(sp);
                    }
                    pc += 1;
                    continue;
                }

                Inst::AssertStart => {
                    if sp == 0 {
                        pc += 1;
                        continue;
                    }
                }

                Inst::AssertEnd => {
                    if sp == text.len() {
                        pc += 1;
                        continue;
                    }
                }

                Inst::AssertWordBoundary => {
                    if is_word_boundary(text, sp) {
                        pc += 1;
                        continue;
                    }
                }

                Inst::AssertNotWordBoundary => {
                    if !is_word_boundary(text, sp) {
                        pc += 1;
                        continue;
                    }
                }

                Inst::Backref(g) => {
                    let g = *g;
                    let s = groups.get(g * 2).and_then(|x| *x);
                    let e = groups.get(g * 2 + 1).and_then(|x| *x);
                    if let (Some(s), Some(e)) = (s, e) {
                        let captured = &text_bytes[s..e];
                        let len = captured.len();
                        if sp + len <= text.len() && &text_bytes[sp..sp + len] == captured {
                            pc += 1;
                            sp += len;
                            continue;
                        }
                    }
                }

                Inst::Lookahead(negative, skip_to) => {
                    let negative = *negative;
                    let skip_to = *skip_to;
                    let body_matched = self.try_lookahead(text, sp, pc + 1, skip_to);
                    if body_matched ^ negative {
                        pc = skip_to;
                        continue;
                    }
                }
            }

            // Backtrack
            if let Some(state) = stack.pop() {
                pc = state.pc;
                sp = state.sp;
                groups = state.groups;
            } else {
                return None;
            }
        }
    }

    /// Try to match sub-program [body_start..body_end) at sp.
    /// Returns true if a Match instruction is reached.
    fn try_lookahead(
        &self,
        text: &str,
        start_sp: usize,
        body_start: usize,
        body_end: usize,
    ) -> bool {
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let mut pc = body_start;
        let mut sp = start_sp;
        let mut steps = 0usize;

        loop {
            steps += 1;
            if steps > 50_000 {
                return false;
            }
            if pc >= body_end {
                if let Some((p, s)) = stack.pop() {
                    pc = p;
                    sp = s;
                    continue;
                }
                return false;
            }

            match &self.program[pc] {
                Inst::Match => return true,
                Inst::Literal(ch) => {
                    if sp < text.len() && char_at(text, sp) == *ch {
                        pc += 1;
                        sp += ch.len_utf8();
                        continue;
                    }
                }
                Inst::AnyChar => {
                    if sp < text.len() {
                        let ch = char_at(text, sp);
                        if ch != '\n' {
                            pc += 1;
                            sp += ch.len_utf8();
                            continue;
                        }
                    }
                }
                Inst::CharClass(cc) => {
                    if sp < text.len() {
                        let ch = char_at(text, sp);
                        if cc.contains(ch) {
                            pc += 1;
                            sp += ch.len_utf8();
                            continue;
                        }
                    }
                }
                Inst::NegCharClass(cc) => {
                    if sp < text.len() {
                        let ch = char_at(text, sp);
                        if !cc.contains(ch) && ch != '\n' {
                            pc += 1;
                            sp += ch.len_utf8();
                            continue;
                        }
                    }
                }
                Inst::Split(a, b) => {
                    stack.push((*b, sp));
                    pc = *a;
                    continue;
                }
                Inst::Jump(t) => {
                    pc = *t;
                    continue;
                }
                Inst::AssertStart => {
                    if sp == 0 {
                        pc += 1;
                        continue;
                    }
                }
                Inst::AssertEnd => {
                    if sp == text.len() {
                        pc += 1;
                        continue;
                    }
                }
                Inst::AssertWordBoundary => {
                    if is_word_boundary(text, sp) {
                        pc += 1;
                        continue;
                    }
                }
                Inst::AssertNotWordBoundary => {
                    if !is_word_boundary(text, sp) {
                        pc += 1;
                        continue;
                    }
                }
                Inst::SaveStart(_) | Inst::SaveEnd(_) => {
                    pc += 1;
                    continue;
                }
                Inst::Backref(_) | Inst::Lookahead(_, _) => {
                    // Unsupported in lookahead body for now
                }
            }

            if let Some((p, s)) = stack.pop() {
                pc = p;
                sp = s;
            } else {
                return false;
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────

fn char_at(text: &str, pos: usize) -> char {
    text[pos..].chars().next().unwrap_or('\0')
}

fn is_word_char_at(text: &str, pos: usize) -> bool {
    if pos >= text.len() {
        return false;
    }
    let ch = char_at(text, pos);
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_word_boundary(text: &str, pos: usize) -> bool {
    let before = if pos > 0 {
        let mut p = pos - 1;
        while p > 0 && !text.is_char_boundary(p) {
            p -= 1;
        }
        is_word_char_at(text, p)
    } else {
        false
    };
    let after = is_word_char_at(text, pos);
    before != after
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Compilation ──

    #[test]
    fn test_compile_literal() {
        let re = Regex::new("abc").unwrap();
        assert!(re.find("abc", 0).is_some());
    }

    #[test]
    fn test_compile_escape() {
        let re = Regex::new(r"a\.b").unwrap();
        assert!(re.find("a.b", 0).is_some());
        assert!(re.find("axb", 0).is_none());
    }

    #[test]
    fn test_compile_char_class() {
        let re = Regex::new("[abc]").unwrap();
        assert!(re.find("b", 0).is_some());
        assert!(re.find("d", 0).is_none());
    }

    #[test]
    fn test_compile_char_class_range() {
        let re = Regex::new("[a-z]").unwrap();
        assert!(re.find("m", 0).is_some());
        assert!(re.find("M", 0).is_none());
    }

    #[test]
    fn test_compile_negated_class() {
        let re = Regex::new("[^0-9]").unwrap();
        assert!(re.find("a", 0).is_some());
        assert!(re.find("5", 0).is_none());
    }

    #[test]
    fn test_compile_quantifiers() {
        Regex::new("a*").unwrap();
        Regex::new("a+").unwrap();
        Regex::new("a?").unwrap();
        Regex::new("a{2,4}").unwrap();
    }

    #[test]
    fn test_compile_alternation() {
        let re = Regex::new("cat|dog").unwrap();
        assert!(re.find("cat", 0).is_some());
        assert!(re.find("dog", 0).is_some());
        assert!(re.find("cow", 0).is_none());
    }

    #[test]
    fn test_compile_groups() {
        Regex::new("(abc)").unwrap();
        Regex::new("(?:abc)").unwrap();
    }

    #[test]
    fn test_compile_inline_mode_flags() {
        // (?i), (?s), (?m), (?i-m) are Oniguruma mode modifiers — must compile without error.
        // We treat them as no-ops (case-sensitivity is not affected; Zenith keywords are uppercase).
        Regex::new("(?i)abc").unwrap();
        Regex::new("(?i)\\b(IF|ELSE-IF)\\b").unwrap();
        Regex::new("(?i-m)\\w+").unwrap();
        // The match is case-sensitive (no-op semantics); uppercase must still match.
        let re = Regex::new("(?i)\\b(ELSE-IF|ELSE|IF)\\b").unwrap();
        assert!(re.find("ELSE-IF x", 0).is_some());
        let m = re.find("ELSE-IF x", 0).unwrap();
        assert_eq!(m.end - m.start, 7, "must match full ELSE-IF (7 chars)");
    }

    #[test]
    fn test_compile_lookahead() {
        Regex::new("(?=abc)").unwrap();
        Regex::new("(?!abc)").unwrap();
    }

    #[test]
    fn test_compile_error() {
        assert!(Regex::new("[").is_err());
        assert!(Regex::new("(").is_err());
    }

    // ── Matching ──

    #[test]
    fn test_literal_match() {
        let re = Regex::new("hello").unwrap();
        let m = re.find("say hello world", 0).unwrap();
        assert_eq!(m.start, 4);
        assert_eq!(m.end, 9);
    }

    #[test]
    fn test_anchors() {
        let re = Regex::new("^hello$").unwrap();
        assert!(re.find("hello", 0).is_some());
        assert!(re.find("say hello", 0).is_none());
        assert!(re.find("hello world", 0).is_none());
    }

    #[test]
    fn test_dot() {
        let re = Regex::new("a.c").unwrap();
        assert!(re.find("abc", 0).is_some());
        assert!(re.find("aXc", 0).is_some());
        assert!(re.find("ac", 0).is_none());
    }

    #[test]
    fn test_char_classes_match() {
        let re = Regex::new("[aeiou]").unwrap();
        let m = re.find("hello", 0).unwrap();
        assert_eq!(m.start, 1);
    }

    #[test]
    fn test_word_digit() {
        let re_w = Regex::new(r"\w+").unwrap();
        assert_eq!(re_w.find("  hello  ", 0).unwrap().start, 2);

        let re_d = Regex::new(r"\d+").unwrap();
        let m = re_d.find("abc123", 0).unwrap();
        assert_eq!(m.start, 3);
        assert_eq!(m.end, 6);
    }

    #[test]
    fn test_quantifier_star() {
        let re = Regex::new("ab*c").unwrap();
        assert!(re.find("ac", 0).is_some());
        assert!(re.find("abc", 0).is_some());
        assert!(re.find("abbc", 0).is_some());
    }

    #[test]
    fn test_quantifier_plus() {
        let re = Regex::new("ab+c").unwrap();
        assert!(re.find("ac", 0).is_none());
        assert!(re.find("abc", 0).is_some());
        assert!(re.find("abbc", 0).is_some());
    }

    #[test]
    fn test_quantifier_question() {
        let re = Regex::new("ab?c").unwrap();
        assert!(re.find("ac", 0).is_some());
        assert!(re.find("abc", 0).is_some());
        assert!(re.find("abbc", 0).is_none());
    }

    #[test]
    fn test_quantifier_counted() {
        let re = Regex::new("a{2,4}").unwrap();
        assert!(re.find("a", 0).is_none());
        assert!(re.find("aa", 0).is_some());
        assert!(re.find("aaa", 0).is_some());
        assert!(re.find("aaaa", 0).is_some());
    }

    #[test]
    fn test_alternation_match() {
        let re = Regex::new("cat|dog|bird").unwrap();
        assert_eq!(re.find("I have a dog", 0).unwrap().start, 9);
        assert_eq!(re.find("a bird flew", 0).unwrap().start, 2);
    }

    #[test]
    fn test_captures() {
        let re = Regex::new(r"(\w+)@(\w+)").unwrap();
        let caps = re.captures("user@host", 0).unwrap();
        assert_eq!(caps.groups[0], Some(Match { start: 0, end: 9 }));
        assert_eq!(caps.groups[1], Some(Match { start: 0, end: 4 }));
        assert_eq!(caps.groups[2], Some(Match { start: 5, end: 9 }));
    }

    #[test]
    fn test_backreference() {
        let re = Regex::new(r"(\w+) \1").unwrap();
        assert!(re.find("hello hello", 0).is_some());
        assert!(re.find("hello world", 0).is_none());
    }

    #[test]
    fn test_word_boundary() {
        let re = Regex::new(r"\bcat\b").unwrap();
        assert!(re.find("the cat sat", 0).is_some());
        assert!(re.find("concatenate", 0).is_none());
    }

    #[test]
    fn test_positive_lookahead() {
        let re = Regex::new("foo(?=bar)").unwrap();
        let m = re.find("foobar", 0).unwrap();
        assert_eq!(m.start, 0);
        assert_eq!(m.end, 3);
        assert!(re.find("foobaz", 0).is_none());
    }

    #[test]
    fn test_negative_lookahead() {
        let re = Regex::new("foo(?!bar)").unwrap();
        assert!(re.find("foobaz", 0).is_some());
        assert!(re.find("foobar", 0).is_none());
    }

    #[test]
    fn test_start_offset() {
        let re = Regex::new("cat").unwrap();
        assert_eq!(re.find("catcat", 0).unwrap().start, 0);
        assert_eq!(re.find("catcat", 1).unwrap().start, 3);
    }

    #[test]
    fn test_utf8() {
        let re = Regex::new("café").unwrap();
        assert!(re.find("I love café!", 0).is_some());
    }

    #[test]
    fn test_no_match() {
        let re = Regex::new("xyz").unwrap();
        assert!(re.find("abc", 0).is_none());
    }

    // ── TextMate-realistic ──

    #[test]
    fn test_keyword_pattern() {
        let re = Regex::new(r"\b(if|else|while|for|return)\b").unwrap();
        let caps = re.captures("  if (x) return;", 0).unwrap();
        assert_eq!(caps.groups[0].unwrap().start, 2);
        assert_eq!(caps.groups[0].unwrap().end, 4);
        assert_eq!(caps.groups[1].unwrap().start, 2);
        assert_eq!(caps.groups[1].unwrap().end, 4);
    }

    #[test]
    fn test_line_comment_pattern() {
        let re = Regex::new("//.*$").unwrap();
        let m = re.find("x = 1; // comment", 0).unwrap();
        assert_eq!(m.start, 7);
        assert_eq!(m.end, 17);
    }

    #[test]
    fn test_escaped_char_pattern() {
        let re = Regex::new(r"\\.").unwrap();
        let m = re.find(r#"say "he\"llo""#, 0).unwrap();
        assert_eq!(m.end - m.start, 2);
    }

    #[test]
    fn test_number_pattern() {
        let re = Regex::new(r"\b\d+(\.\d+)?\b").unwrap();
        let caps = re.captures("x = 3.14;", 0).unwrap();
        assert_eq!(caps.groups[0].unwrap().start, 4);
        assert_eq!(caps.groups[0].unwrap().end, 8);
        assert!(caps.groups[1].is_some());
    }

    // ── Safety ──

    #[test]
    fn test_runaway_protection() {
        let re = Regex::new("(a+)+b").unwrap();
        assert!(re.find("aaaaaaaaaa", 0).is_none());
    }
}
