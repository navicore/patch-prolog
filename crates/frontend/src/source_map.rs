//! Maps byte offsets (the stable position in a `Span`) back to human
//! line/column. Built on demand from the source text a consumer already
//! holds — positions are not threaded through parser return types.

/// Line/column resolver for a single source buffer.
pub struct SourceMap<'a> {
    src: &'a str,
    /// Byte offset of the start of each line (line 0 starts at 0).
    line_starts: Vec<u32>,
}

impl<'a> SourceMap<'a> {
    pub fn new(src: &'a str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        SourceMap { src, line_starts }
    }

    /// 1-based line and column (column counts characters from the line start).
    /// For human-facing `file:line:col` rendering.
    pub fn line_col(&self, offset: u32) -> (usize, usize) {
        let line = self.line_index(offset);
        let start = self.line_starts[line] as usize;
        let col = self
            .src
            .get(start..offset as usize)
            .map(|s| s.chars().count())
            .unwrap_or(0);
        (line + 1, col + 1)
    }

    /// 0-based line and 0-based UTF-16 column — an LSP `Position`.
    pub fn utf16_position(&self, offset: u32) -> (u32, u32) {
        let line = self.line_index(offset);
        let start = self.line_starts[line] as usize;
        let col: u32 = self
            .src
            .get(start..offset as usize)
            .map(|s| s.chars().map(|c| c.len_utf16() as u32).sum())
            .unwrap_or(0);
        (line as u32, col)
    }

    fn line_index(&self, offset: u32) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_line_and_col() {
        let sm = SourceMap::new("ab\ncde\nf");
        assert_eq!(sm.line_col(0), (1, 1)); // 'a'
        assert_eq!(sm.line_col(1), (1, 2)); // 'b'
        assert_eq!(sm.line_col(3), (2, 1)); // 'c'
        assert_eq!(sm.line_col(5), (2, 3)); // 'e'
        assert_eq!(sm.line_col(7), (3, 1)); // 'f'
    }

    #[test]
    fn utf16_columns_count_code_units() {
        // '😀' is 4 UTF-8 bytes, 2 UTF-16 units.
        let sm = SourceMap::new("😀x");
        assert_eq!(sm.utf16_position(0), (0, 0)); // before emoji
        assert_eq!(sm.utf16_position(4), (0, 2)); // 'x', after the 2 UTF-16 units
    }
}
