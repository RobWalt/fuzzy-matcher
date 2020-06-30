use crate::skim::Movement::{Match, Skip};
///! The fuzzy matching algorithm used by skim
///!
///! # Example:
///! ```edition2018
///! use fuzzy_matcher::FuzzyMatcher;
///! use fuzzy_matcher::skim::SkimMatcherV2;
///!
///! let matcher = SkimMatcherV2::default();
///! assert_eq!(None, matcher.fuzzy_match("abc", "abx"));
///! assert!(matcher.fuzzy_match("axbycz", "abc").is_some());
///! assert!(matcher.fuzzy_match("axbycz", "xyz").is_some());
///!
///! let (score, indices) = matcher.fuzzy_indices("axbycz", "abc").unwrap();
///! assert_eq!(indices, [0, 2, 4]);
///! ```
use crate::{FuzzyMatcher, IndexType, ScoreType};
use std::cmp::max;
use std::ptr;

const BONUS_MATCHED: ScoreType = 4;
const BONUS_CASE_MATCH: ScoreType = 4;
const BONUS_UPPER_MATCH: ScoreType = 6;
const BONUS_ADJACENCY: ScoreType = 10;
const BONUS_SEPARATOR: ScoreType = 8;
const BONUS_CAMEL: ScoreType = 8;
const PENALTY_CASE_UNMATCHED: ScoreType = -1;
const PENALTY_LEADING: ScoreType = -6; // penalty applied for every letter before the first match
const PENALTY_MAX_LEADING: ScoreType = -18; // maxing penalty for leading letters
const PENALTY_UNMATCHED: ScoreType = -2;

pub struct SkimMatcher {}

impl Default for SkimMatcher {
    fn default() -> Self {
        Self {}
    }
}

// The V1 matcher is based on ForrestTheWoods's post
// https://www.forrestthewoods.com/blog/reverse_engineering_sublime_texts_fuzzy_match/
impl FuzzyMatcher for SkimMatcher {
    fn fuzzy_indices(&self, choice: &str, pattern: &str) -> Option<(ScoreType, Vec<IndexType>)> {
        fuzzy_indices(choice, pattern)
    }

    fn fuzzy_match(&self, choice: &str, pattern: &str) -> Option<ScoreType> {
        fuzzy_match(choice, pattern)
    }
}

pub fn fuzzy_match(choice: &str, pattern: &str) -> Option<ScoreType> {
    if pattern.is_empty() {
        return Some(0);
    }

    let scores = build_graph(choice, pattern)?;

    let last_row = &scores[scores.len() - 1];
    let (_, &MatchingStatus { final_score, .. }) = last_row
        .iter()
        .enumerate()
        .max_by_key(|&(_, x)| x.final_score)
        .expect("fuzzy_indices failed to iterate over last_row");
    Some(final_score)
}

pub fn fuzzy_indices(choice: &str, pattern: &str) -> Option<(ScoreType, Vec<IndexType>)> {
    if pattern.is_empty() {
        return Some((0, Vec::new()));
    }

    let mut picked = vec![];
    let scores = build_graph(choice, pattern)?;

    let last_row = &scores[scores.len() - 1];
    let (mut next_col, &MatchingStatus { final_score, .. }) = last_row
        .iter()
        .enumerate()
        .max_by_key(|&(_, x)| x.final_score)
        .expect("fuzzy_indices failed to iterate over last_row");
    let mut pat_idx = scores.len() as i64 - 1;
    while pat_idx >= 0 {
        let status = scores[pat_idx as usize][next_col];
        next_col = status.back_ref as usize;
        picked.push(status.idx);
        pat_idx -= 1;
    }
    picked.reverse();
    Some((final_score, picked))
}

#[derive(Clone, Copy, Debug)]
struct MatchingStatus {
    pub idx: IndexType,
    pub score: ScoreType,
    pub final_score: ScoreType,
    pub adj_num: IndexType,
    pub back_ref: IndexType,
}

impl Default for MatchingStatus {
    fn default() -> Self {
        MatchingStatus {
            idx: 0,
            score: 0,
            final_score: 0,
            adj_num: 1,
            back_ref: 0,
        }
    }
}

fn build_graph(choice: &str, pattern: &str) -> Option<Vec<Vec<MatchingStatus>>> {
    let mut scores = vec![];

    let mut match_start_idx = 0; // to ensure that the pushed char are able to match the pattern
    let mut pat_prev_ch = '\0';

    // initialize the match positions and inline scores
    for (pat_idx, pat_ch) in pattern.chars().enumerate() {
        let mut vec = vec![];
        let mut choice_prev_ch = '\0';
        for (idx, ch) in choice.chars().enumerate() {
            if ch.to_ascii_lowercase() == pat_ch.to_ascii_lowercase() && idx >= match_start_idx {
                let score = fuzzy_score(
                    ch,
                    idx as IndexType,
                    choice_prev_ch,
                    pat_ch,
                    pat_idx as IndexType,
                    pat_prev_ch,
                );
                vec.push(MatchingStatus {
                    idx: idx as IndexType,
                    score,
                    final_score: score,
                    adj_num: 1,
                    back_ref: 0,
                });
            }
            choice_prev_ch = ch;
        }

        if vec.is_empty() {
            // not matched
            return None;
        }
        match_start_idx = vec[0].idx as usize + 1;
        scores.push(vec);
        pat_prev_ch = pat_ch;
    }

    // calculate max scores considering adjacent characters
    for pat_idx in 1..scores.len() {
        let (first_half, last_half) = scores.split_at_mut(pat_idx);

        let prev_row = &first_half[first_half.len() - 1];
        let cur_row = &mut last_half[0];

        for idx in 0..cur_row.len() {
            let next = cur_row[idx];
            let prev = if idx > 0 {
                cur_row[idx - 1]
            } else {
                MatchingStatus::default()
            };

            let mut score_before_idx = prev.final_score - prev.score + next.score;
            score_before_idx += PENALTY_UNMATCHED * ((next.idx - prev.idx) as ScoreType);
            score_before_idx -= if prev.adj_num == 0 {
                BONUS_ADJACENCY
            } else {
                0
            };

            let (back_ref, score, adj_num) = prev_row
                .iter()
                .enumerate()
                .take_while(|&(_, &MatchingStatus { idx, .. })| idx < next.idx)
                .skip_while(|&(_, &MatchingStatus { idx, .. })| idx < prev.idx)
                .map(|(back_ref, cur)| {
                    let adj_num = next.idx - cur.idx - 1;
                    let mut final_score = cur.final_score + next.score;
                    final_score += if adj_num == 0 {
                        BONUS_ADJACENCY
                    } else {
                        PENALTY_UNMATCHED * adj_num as ScoreType
                    };
                    (back_ref, final_score, adj_num)
                })
                .max_by_key(|&(_, x, _)| x)
                .unwrap_or((prev.back_ref as usize, score_before_idx, prev.adj_num));

            cur_row[idx] = if idx > 0 && score < score_before_idx {
                MatchingStatus {
                    final_score: score_before_idx,
                    back_ref: prev.back_ref,
                    adj_num,
                    ..next
                }
            } else {
                MatchingStatus {
                    final_score: score,
                    back_ref: back_ref as IndexType,
                    adj_num,
                    ..next
                }
            };
        }
    }

    Some(scores)
}

// judge how many scores the current index should get
fn fuzzy_score(
    choice_ch: char,
    choice_idx: IndexType,
    choice_prev_ch: char,
    pat_ch: char,
    pat_idx: IndexType,
    _pat_prev_ch: char,
) -> ScoreType {
    let mut score = BONUS_MATCHED;

    let choice_prev_ch_type = CharType::of(choice_prev_ch);
    let choice_role = CharRole::of(choice_prev_ch, choice_ch);

    if pat_ch == choice_ch {
        if pat_ch.is_uppercase() {
            score += BONUS_UPPER_MATCH;
        } else {
            score += BONUS_CASE_MATCH;
        }
    } else {
        score += PENALTY_CASE_UNMATCHED;
    }

    // apply bonus for camelCases
    if choice_role == CharRole::Head
        || choice_role == CharRole::Break
        || choice_role == CharRole::Camel
    {
        score += BONUS_CAMEL;
    }

    // apply bonus for matches after a separator
    if choice_prev_ch_type == CharType::HardSep || choice_prev_ch_type == CharType::SoftSep {
        score += BONUS_SEPARATOR;
    }

    if pat_idx == 0 {
        score += max(
            (choice_idx as ScoreType) * PENALTY_LEADING,
            PENALTY_MAX_LEADING,
        );
    }

    score
}

#[derive(Copy, Clone)]
pub struct SkimScoreConfig {
    pub score_match: i32,
    pub gap_start: i32,
    pub gap_extension: i32,

    /// The first character in the typed pattern usually has more significance
    /// than the rest so it's important that it appears at special positions where
    /// bonus points are given. e.g. "to-go" vs. "ongoing" on "og" or on "ogo".
    /// The amount of the extra bonus should be limited so that the gap penalty is
    /// still respected.
    pub bonus_first_char_multiplier: i32,

    /// We prefer matches at the beginning of a word, but the bonus should not be
    /// too great to prevent the longer acronym matches from always winning over
    /// shorter fuzzy matches. The bonus point here was specifically chosen that
    /// the bonus is cancelled when the gap between the acronyms grows over
    /// 8 characters, which is approximately the average length of the words found
    /// in web2 dictionary and my file system.
    pub bonus_head: i32,

    /// Just like bonus_head, but its breakage of word is not that strong, so it should
    /// be slighter less then bonus_head
    pub bonus_break: i32,

    /// Edge-triggered bonus for matches in camelCase words.
    /// Compared to word-boundary case, they don't accompany single-character gaps
    /// (e.g. FooBar vs. foo-bar), so we deduct bonus point accordingly.
    pub bonus_camel: i32,

    /// Minimum bonus point given to characters in consecutive chunks.
    /// Note that bonus points for consecutive matches shouldn't have needed if we
    /// used fixed match score as in the original algorithm.
    pub bonus_consecutive: i32,

    /// Skim will match case-sensitively if the pattern contains ASCII upper case,
    /// If case of case insensitive match, the penalty will be given to case mismatch
    pub penalty_case_mismatch: i32,
}

impl Default for SkimScoreConfig {
    fn default() -> Self {
        let score_match = 16;
        let gap_start = -3;
        let gap_extension = -1;
        let bonus_first_char_multiplier = 2;

        Self {
            score_match,
            gap_start,
            gap_extension,
            bonus_first_char_multiplier,
            bonus_head: score_match / 2,
            bonus_break: score_match / 2 + gap_extension,
            bonus_camel: score_match / 2 + 2 * gap_extension,
            bonus_consecutive: -(gap_start + gap_extension),
            penalty_case_mismatch: gap_extension * 2,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
enum Movement {
    Match,
    Skip,
}

/// Inner state of the score matrix
#[derive(Debug, Copy, Clone)]
struct MatrixCell {
    pub movement: Movement,
    pub score: i32, // The max score of align pattern[..i] & choice[..j]
}

const MATRIX_CELL_NEG_INFINITY: i32 = std::i16::MIN as i32;
impl Default for MatrixCell {
    fn default() -> Self {
        Self {
            movement: Skip,
            score: MATRIX_CELL_NEG_INFINITY,
        }
    }
}

/// Simulate a 1-D vector as 2-D matrix
struct ScoreMatrix<'a> {
    matrix: &'a mut [MatrixCell],
    pub rows: usize,
    pub cols: usize,
}

impl<'a> ScoreMatrix<'a> {
    /// given a matrix, extend it to be (rows x cols) and fill in as init_val
    pub fn new(matrix: &'a mut Vec<MatrixCell>, rows: usize, cols: usize) -> Self {
        matrix.resize(rows * cols, MatrixCell::default());
        ScoreMatrix { matrix, rows, cols }
    }

    #[inline]
    fn get_score(&self, row: usize, col: usize) -> i32 {
        self.matrix[row * self.cols + col].score
    }

    #[inline]
    fn get_movement(&self, row: usize, col: usize) -> Movement {
        self.matrix[row * self.cols + col].movement
    }

    #[inline]
    fn set_score(&mut self, row: usize, col: usize, score: i32) {
        self.matrix[row * self.cols + col].score = score;
    }

    #[inline]
    fn set_movement(&mut self, row: usize, col: usize, movement: Movement) {
        self.matrix[row * self.cols + col].movement = movement;
    }

    fn get_row(&self, row: usize) -> &[MatrixCell] {
        let start = row * self.cols;
        &self.matrix[start..start + self.cols]
    }
}

/// We categorize characters into types:
///
/// - Empty(E): the start of string
/// - Upper(U): the ascii upper case
/// - lower(L): the ascii lower case & other unicode characters
/// - number(N): ascii number
/// - hard separator(S): clearly separate the content: ` ` `/` `\` `|` `(` `) `[` `]` `{` `}`
/// - soft separator(s): other ascii punctuation, e.g. `!` `"` `#` `$`, ...
#[derive(Debug, PartialEq, Copy, Clone)]
enum CharType {
    Empty,
    Upper,
    Lower,
    Number,
    HardSep,
    SoftSep,
}

impl CharType {
    pub fn of(ch: char) -> Self {
        if ch == '\0' {
            CharType::Empty
        } else if ch == ' '
            || ch == '/'
            || ch == '\\'
            || ch == '|'
            || ch == '('
            || ch == ')'
            || ch == '['
            || ch == ']'
            || ch == '{'
            || ch == '}'
        {
            CharType::HardSep
        } else if ch.is_ascii_punctuation() {
            CharType::SoftSep
        } else if ch.is_ascii_digit() {
            CharType::Number
        } else if ch.is_ascii_uppercase() {
            CharType::Upper
        } else {
            CharType::Lower
        }
    }
}

/// Ref: https://github.com/llvm-mirror/clang-tools-extra/blob/master/clangd/FuzzyMatch.cpp
///
///
/// ```text
/// +-----------+--------------+-------+
/// | Example   | Chars | Type | Role  |
/// +-----------+--------------+-------+
/// | (f)oo     | ^fo   | Ell  | Head  |
/// | (F)oo     | ^Fo   | EUl  | Head  |
/// | Foo/(B)ar | /Ba   | SUl  | Head  |
/// | Foo/(b)ar | /ba   | Sll  | Head  |
/// | Foo.(B)ar | .Ba   | SUl  | Break |
/// | Foo(B)ar  | oBa   | lUl  | Camel |
/// | 123(B)ar  | 3Ba   | nUl  | Camel |
/// | F(o)oBar  | Foo   | Ull  | Tail  |
/// | H(T)TP    | HTT   | UUU  | Tail  |
/// | others    |       |      | Tail  |
/// +-----------+--------------+-------+
#[derive(Debug, PartialEq, Copy, Clone)]
enum CharRole {
    Head,
    Tail,
    Camel,
    Break,
}

impl CharRole {
    pub fn of(prev: char, cur: char) -> Self {
        Self::of_type(CharType::of(prev), CharType::of(cur))
    }
    pub fn of_type(prev: CharType, cur: CharType) -> Self {
        match (prev, cur) {
            (CharType::Empty, _) | (CharType::HardSep, _) => CharRole::Head,
            (CharType::SoftSep, _) => CharRole::Break,
            (CharType::Lower, CharType::Upper) | (CharType::Number, CharType::Upper) => {
                CharRole::Camel
            }
            _ => CharRole::Tail,
        }
    }
}

use crate::util::{char_equal, cheap_matches};
use std::cell::RefCell;
use thread_local::CachedThreadLocal;

#[derive(Eq, PartialEq, Debug, Copy, Clone)]
enum CaseMatching {
    Respect,
    Ignore,
    Smart,
}

/// Fuzzy matching is a sub problem is sequence alignment.
/// Specifically what we'd like to implement is sequence alignment with affine gap penalty.
/// Ref: https://www.cs.cmu.edu/~ckingsf/bioinfo-lectures/gaps.pdf
///
/// Given `pattern`(i) and `choice`(j), we'll maintain 2 score matrix:
///
/// ```text
/// M[i][j] = match(i, j) + max(M[i-1][j-1] + consecutive, P[i-1][j-1])
/// M[i][j] = -infinity if p[i][j] do not match
///
/// M[i][j] means the score of best alignment of p[..=i] and c[..=j] ending with match/mismatch e.g.:
///
/// c: [.........]b
/// p: [.........]b
///
/// So that p[..=i-1] and c[..=j-1] could be any alignment
///
/// P[i][j] = max(M[i][j-k]-gap(k)) for k in 1..j
///
/// P[i][j] means the score of best alignment of p[..=i] and c[..=j] where c[j] is not matched.
/// So that we need to search through all the previous matches, and calculate the gap.
///
///   (j-k)--.   j
/// c: [....]bcdef
/// p: [....]b----
///          i
/// ```
///
/// Note that the above is O(n^3) in the worst case. However the above algorithm uses a general gap
/// penalty, but we use affine gap: `gap = gap_start + k * gap_extend` where:
/// - u: the cost of starting of gap
/// - v: the cost of extending a gap by one more space.
///
/// So that we could optimize the algorithm by:
///
/// ```text
/// P[i][j] = max(gap_start + gap_extend + M[i][j-1], gap_extend + P[i][j-1])
/// ```
///
/// In summary:
///
/// ```text
/// M[i][j] = match(i, j) + max(M[i-1][j-1] + consecutive, P[i-1][j-1])
/// M[i][j] = -infinity if p[i] and c[j] do not match
/// P[i][j] = max(gap_start + gap_extend + M[i][j-1], gap_extend + P[i][j-1])
/// ```
pub struct SkimMatcherV2 {
    score_config: SkimScoreConfig,
    element_limit: usize,
    case: CaseMatching,
    use_cache: bool,

    m_cache: CachedThreadLocal<RefCell<Vec<MatrixCell>>>,
    p_cache: CachedThreadLocal<RefCell<Vec<MatrixCell>>>,
}

impl Default for SkimMatcherV2 {
    fn default() -> Self {
        Self {
            score_config: SkimScoreConfig::default(),
            element_limit: 0,
            case: CaseMatching::Smart,
            use_cache: true,

            m_cache: CachedThreadLocal::new(),
            p_cache: CachedThreadLocal::new(),
        }
    }
}

impl SkimMatcherV2 {
    pub fn score_config(mut self, score_config: SkimScoreConfig) -> Self {
        self.score_config = score_config;
        self
    }

    pub fn element_limit(mut self, elements: usize) -> Self {
        self.element_limit = elements;
        self
    }

    pub fn ignore_case(mut self) -> Self {
        self.case = CaseMatching::Ignore;
        self
    }

    pub fn smart_case(mut self) -> Self {
        self.case = CaseMatching::Smart;
        self
    }

    pub fn respect_case(mut self) -> Self {
        self.case = CaseMatching::Respect;
        self
    }

    pub fn use_cache(mut self, use_cache: bool) -> Self {
        self.use_cache = use_cache;
        self
    }

    /// Build the score matrix using the algorithm described above
    fn build_score_matrix(
        &self,
        m: &mut ScoreMatrix,
        p: &mut ScoreMatrix,
        choice: &str,
        pattern: &str,
        compressed: bool,
        case_sensitive: bool,
    ) {
        for i in 0..m.rows {
            m.set_score(i, 0, MATRIX_CELL_NEG_INFINITY);
            m.set_movement(i, 0, Movement::Skip);
        }

        for j in 0..m.cols {
            m.set_score(0, j, MATRIX_CELL_NEG_INFINITY);
            m.set_movement(0, j, Movement::Skip);
        }

        for i in 0..p.rows {
            p.set_score(i, 0, MATRIX_CELL_NEG_INFINITY);
            p.set_movement(i, 0, Movement::Skip);
        }

        // p[0][j]: the score of best alignment of p[] and c[..=j] where c[j] is not matched
        for j in 0..p.cols {
            p.set_score(0, j, self.score_config.gap_extension);
            p.set_movement(0, j, Movement::Skip);
        }

        // update the matrix;
        for (i, p_ch) in pattern.chars().enumerate() {
            let mut prev_ch = '\0';

            for (j, c_ch) in choice.chars().enumerate() {
                let row = self.adjust_row_idx(i + 1, compressed);
                let row_prev = self.adjust_row_idx(i, compressed);
                let col = j + 1;
                let col_prev = j;

                // update M matrix
                // M[i][j] = match(i, j) + max(M[i-1][j-1], P[i-1][j-1])
                if let Some(match_score) =
                    self.calculate_match_score(prev_ch, c_ch, p_ch, i, j, case_sensitive)
                {
                    let prev_match_score = m.get_score(row_prev, col_prev);
                    let prev_skip_score = p.get_score(row_prev, col_prev);
                    if prev_match_score >= prev_skip_score {
                        m.set_movement(row, col, Movement::Match);
                    }
                    m.set_score(
                        row,
                        col,
                        (match_score as i32)
                            + max(
                                prev_match_score + self.score_config.bonus_consecutive,
                                prev_skip_score,
                            ),
                    );
                } else {
                    m.set_score(row, col, MATRIX_CELL_NEG_INFINITY);
                    m.set_movement(row, col, Movement::Skip);
                }

                // update P matrix
                // P[i][j] = max(gap_start + gap_extend + M[i][j-1], gap_extend + P[i][j-1])
                let prev_match_score = self.score_config.gap_start
                    + self.score_config.gap_extension
                    + m.get_score(row, col_prev);
                let prev_skip_score = self.score_config.gap_extension + p.get_score(row, col_prev);
                if prev_match_score >= prev_skip_score {
                    p.set_score(row, col, prev_match_score);
                    p.set_movement(row, col, Movement::Match);
                } else {
                    p.set_score(row, col, prev_skip_score);
                    p.set_movement(row, col, Movement::Skip);
                }

                prev_ch = c_ch;
            }
        }
    }

    /// In case we don't need to backtrack the matching indices, we could use only 2 rows for the
    /// matrix, this function could be used to rotate accessing these two rows.
    fn adjust_row_idx(&self, row_idx: usize, compressed: bool) -> usize {
        if compressed {
            row_idx & 1
        } else {
            row_idx
        }
    }

    /// Calculate the matching score of the characters
    /// return None if not matched.
    fn calculate_match_score(
        &self,
        prev_ch: char,
        c: char,
        p: char,
        c_idx: usize,
        _p_idx: usize,
        case_sensitive: bool,
    ) -> Option<u16> {
        if !char_equal(c, p, case_sensitive) {
            return None;
        }

        let score = self.score_config.score_match;

        // check bonus for start of camel case, etc.
        let prev_ch_type = CharType::of(prev_ch);
        let ch_type = CharType::of(c);
        let mut bonus = self.in_place_bonus(prev_ch_type, ch_type);

        // bonus for matching the start of the whole choice string
        if c_idx == 0 {
            bonus *= self.score_config.bonus_first_char_multiplier;
        }

        // penalty on case mismatch
        if !case_sensitive && p != c {
            bonus += self.score_config.penalty_case_mismatch;
        }

        Some(max(0, score + bonus) as u16)
    }

    fn in_place_bonus(&self, prev_char_type: CharType, char_type: CharType) -> i32 {
        match CharRole::of_type(prev_char_type, char_type) {
            CharRole::Head => self.score_config.bonus_head,
            CharRole::Camel => self.score_config.bonus_camel,
            CharRole::Break => self.score_config.bonus_break,
            CharRole::Tail => 0,
        }
    }

    fn contains_upper(&self, string: &str) -> bool {
        for ch in string.chars() {
            if ch.is_ascii_uppercase() {
                return true;
            }
        }

        false
    }

    pub fn fuzzy(
        &self,
        choice: &str,
        pattern: &str,
        with_pos: bool,
    ) -> Option<(ScoreType, Vec<IndexType>)> {
        if pattern.is_empty() {
            return Some((0, Vec::new()));
        }

        let case_sensitive = match self.case {
            CaseMatching::Respect => true,
            CaseMatching::Ignore => false,
            CaseMatching::Smart => self.contains_upper(pattern),
        };

        let compressed = !with_pos;

        if !cheap_matches(choice, pattern, case_sensitive) {
            return None;
        }

        if pattern.is_empty() {
            return Some((0, Vec::new()));
        }

        let cols = choice.chars().count() + 1;
        let num_char_pattern = pattern.chars().count();
        let rows = if compressed { 2 } else { num_char_pattern + 1 };

        if self.element_limit > 0 && self.element_limit < rows * cols {
            return self.simple_match(choice, pattern, case_sensitive, with_pos);
        }

        // initialize the score matrix
        let mut m = self
            .m_cache
            .get_or(|| RefCell::new(Vec::new()))
            .borrow_mut();
        let mut m = ScoreMatrix::new(&mut m, rows, cols);
        let mut p = self
            .p_cache
            .get_or(|| RefCell::new(Vec::new()))
            .borrow_mut();
        let mut p = ScoreMatrix::new(&mut p, rows, cols);

        self.build_score_matrix(&mut m, &mut p, choice, pattern, compressed, case_sensitive);
        let last_row = m.get_row(self.adjust_row_idx(num_char_pattern, compressed));
        let (pat_idx, &MatrixCell { score, .. }) = last_row
            .iter()
            .enumerate()
            .max_by_key(|&(_, x)| x.score)
            .expect("fuzzy_matcher failed to iterate over last_row");

        let mut positions = if with_pos { Vec::with_capacity(num_char_pattern)} else {Vec::new()};
        if with_pos {
            let mut i = m.rows - 1;
            let mut j = pat_idx;
            let mut matrix = &m;
            let mut current_move = Match;
            while i > 0 && j > 0 {
                if current_move == Match {
                    positions.push((j - 1) as IndexType);
                }

                current_move = matrix.get_movement(i, j);
                if ptr::eq(matrix, &m) {
                    i -= 1;
                }

                j -= 1;

                matrix = match current_move {
                    Match => &m,
                    Skip => &p,
                };
            }
            positions.reverse();
        }

        if !self.use_cache {
            // drop the allocated memory
            self.m_cache.get().map(|cell| cell.replace(vec![]));
            self.p_cache.get().map(|cell| cell.replace(vec![]));
        }

        Some((score as ScoreType, positions))
    }

    /// Borrowed from fzf v1, if the memory limit exceeded, fallback to simple linear search
    pub fn simple_match(
        &self,
        choice: &str,
        pattern: &str,
        case_sensitive: bool,
        with_pos: bool,
    ) -> Option<(ScoreType, Vec<IndexType>)> {
        let mut choice_iter = choice.char_indices().peekable();
        let mut pattern_iter = pattern.chars().peekable();
        let mut o_start_byte = None;

        // scan forward to find the first match of whole pattern
        let mut start_chars = 0;
        while choice_iter.peek().is_some() && pattern_iter.peek().is_some() {
            let (byte_idx, c) = choice_iter.next().unwrap();
            match pattern_iter.peek() {
                Some(&p) => {
                    if char_equal(c, p, case_sensitive) {
                        let _ = pattern_iter.next();
                        o_start_byte = o_start_byte.or(Some(byte_idx));
                    }
                }
                None => break,
            }

            if o_start_byte.is_none() {
                start_chars += 1;
            }
        }

        if pattern_iter.peek().is_some() {
            return None;
        }

        let start_byte = o_start_byte.unwrap_or(0);
        let end_byte = choice_iter
            .next()
            .map(|(idx, _)| idx)
            .unwrap_or_else(|| choice.len());

        // scan backward to find the first match of whole pattern
        let mut o_nearest_start_byte = None;
        let mut pattern_iter = pattern.chars().rev().peekable();
        for (idx, c) in choice[start_byte..end_byte].char_indices().rev() {
            match pattern_iter.peek() {
                Some(&p) => {
                    if char_equal(c, p, case_sensitive) {
                        let _ = pattern_iter.next();
                        o_nearest_start_byte = Some(idx);
                    }
                }
                None => break,
            }
        }

        let start_byte = start_byte + o_nearest_start_byte.unwrap_or(0);
        Some(self.calculate_score_with_pos(
            choice,
            pattern,
            start_byte,
            end_byte,
            start_chars,
            case_sensitive,
            with_pos,
        ))
    }

    fn calculate_score_with_pos(
        &self,
        choice: &str,
        pattern: &str,
        start_bytes: usize,
        end_bytes: usize,
        start_chars: usize,
        case_sensitive: bool,
        with_pos: bool,
    ) -> (ScoreType, Vec<IndexType>) {
        let mut pos = Vec::new();

        let choice_iter = choice[start_bytes..end_bytes].chars().enumerate();
        let mut pattern_iter = pattern.chars().enumerate().peekable();

        // unfortunately we could not get the the character before the first character's(for performance)
        // so we tread them as NonWord
        let mut prev_ch = '\0';

        let mut score: i32 = 0;
        let mut in_gap = false;
        let mut consecutive = 0;

        for (c_idx, c) in choice_iter {
            let op = pattern_iter.peek();
            if op.is_none() {
                break;
            }

            let (p_idx, p) = *op.unwrap();

            if let Some(match_score) = self.calculate_match_score(
                prev_ch,
                c,
                p,
                c_idx + start_chars,
                p_idx,
                case_sensitive,
            ) {
                if with_pos {
                    pos.push((c_idx + start_chars) as IndexType);
                }
                score += match_score as i32;
                score += consecutive * self.score_config.bonus_consecutive;

                in_gap = false;
                consecutive += 1;
                let _ = pattern_iter.next();
            } else {
                if !in_gap {
                    score += self.score_config.gap_start;
                }

                score += self.score_config.gap_extension;
                in_gap = true;
                consecutive = 0;
            }

            prev_ch = c;
        }

        (score as ScoreType, pos)
    }
}

impl FuzzyMatcher for SkimMatcherV2 {
    fn fuzzy_indices(&self, choice: &str, pattern: &str) -> Option<(ScoreType, Vec<IndexType>)> {
        self.fuzzy(choice, pattern, true)
    }

    fn fuzzy_match(&self, choice: &str, pattern: &str) -> Option<ScoreType> {
        self.fuzzy(choice, pattern, false).map(|(score, _)| score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{assert_order, wrap_matches};

    fn wrap_fuzzy_match(matcher: &dyn FuzzyMatcher, line: &str, pattern: &str) -> Option<String> {
        let (_score, indices) = matcher.fuzzy_indices(line, pattern)?;
        Some(wrap_matches(line, &indices))
    }

    #[test]
    fn test_match_or_not() {
        let matcher = SkimMatcher::default();
        assert_eq!(Some(0), matcher.fuzzy_match("", ""));
        assert_eq!(Some(0), matcher.fuzzy_match("abcdefaghi", ""));
        assert_eq!(None, matcher.fuzzy_match("", "a"));
        assert_eq!(None, matcher.fuzzy_match("abcdefaghi", "中"));
        assert_eq!(None, matcher.fuzzy_match("abc", "abx"));
        assert!(matcher.fuzzy_match("axbycz", "abc").is_some());
        assert!(matcher.fuzzy_match("axbycz", "xyz").is_some());

        assert_eq!(
            "[a]x[b]y[c]z",
            &wrap_fuzzy_match(&matcher, "axbycz", "abc").unwrap()
        );
        assert_eq!(
            "a[x]b[y]c[z]",
            &wrap_fuzzy_match(&matcher, "axbycz", "xyz").unwrap()
        );
        assert_eq!(
            "[H]ello, [世]界",
            &wrap_fuzzy_match(&matcher, "Hello, 世界", "H世").unwrap()
        );
    }

    #[test]
    fn test_match_quality() {
        let matcher = SkimMatcher::default();

        // initials
        assert_order(&matcher, "ab", &["ab", "aoo_boo", "acb"]);
        assert_order(&matcher, "CC", &["CamelCase", "camelCase", "camelcase"]);
        assert_order(&matcher, "cC", &["camelCase", "CamelCase", "camelcase"]);
        assert_order(
            &matcher,
            "cc",
            &[
                "camel case",
                "camelCase",
                "camelcase",
                "CamelCase",
                "camel ace",
            ],
        );
        assert_order(
            &matcher,
            "Da.Te",
            &["Data.Text", "Data.Text.Lazy", "Data.Aeson.Encoding.text"],
        );
        // prefix
        assert_order(&matcher, "is", &["isIEEE", "inSuf"]);
        // shorter
        assert_order(&matcher, "ma", &["map", "many", "maximum"]);
        assert_order(&matcher, "print", &["printf", "sprintf"]);
        // score(PRINT) = kMinScore
        assert_order(&matcher, "ast", &["ast", "AST", "INT_FAST16_MAX"]);
        // score(PRINT) > kMinScore
        assert_order(&matcher, "Int", &["int", "INT", "PRINT"]);
    }

    #[test]
    fn test_match_or_not_simple() {
        let matcher = SkimMatcherV2::default();
        assert_eq!(
            matcher
                .simple_match("axbycz", "xyz", false, true)
                .unwrap()
                .1,
            vec![1, 3, 5]
        );

        assert_eq!(
            matcher.simple_match("", "", false, false),
            Some((0, vec![]))
        );
        assert_eq!(
            matcher.simple_match("abcdefaghi", "", false, false),
            Some((0, vec![]))
        );
        assert_eq!(matcher.simple_match("", "a", false, false), None);
        assert_eq!(
            matcher.simple_match("abcdefaghi", "中", false, false,),
            None
        );
        assert_eq!(matcher.simple_match("abc", "abx", false, false,), None);
        assert_eq!(
            matcher
                .simple_match("axbycz", "abc", false, true)
                .unwrap()
                .1,
            vec![0, 2, 4]
        );
        assert_eq!(
            matcher
                .simple_match("axbycz", "xyz", false, true)
                .unwrap()
                .1,
            vec![1, 3, 5]
        );
        assert_eq!(
            matcher
                .simple_match("Hello, 世界", "H世", false, true)
                .unwrap()
                .1,
            vec![0, 7]
        );
    }

    #[test]
    fn test_match_or_not_v2() {
        let matcher = SkimMatcherV2::default();

        assert_eq!(matcher.fuzzy_match("", ""), Some(0));
        assert_eq!(matcher.fuzzy_match("abcdefaghi", ""), Some(0));
        assert_eq!(matcher.fuzzy_match("", "a"), None);
        assert_eq!(matcher.fuzzy_match("abcdefaghi", "中"), None);
        assert_eq!(matcher.fuzzy_match("abc", "abx"), None);
        assert!(matcher.fuzzy_match("axbycz", "abc").is_some());
        assert!(matcher.fuzzy_match("axbycz", "xyz").is_some());

        assert_eq!(
            &wrap_fuzzy_match(&matcher, "axbycz", "abc").unwrap(),
            "[a]x[b]y[c]z"
        );
        assert_eq!(
            &wrap_fuzzy_match(&matcher, "axbycz", "xyz").unwrap(),
            "a[x]b[y]c[z]"
        );
        assert_eq!(
            &wrap_fuzzy_match(&matcher, "Hello, 世界", "H世").unwrap(),
            "[H]ello, [世]界"
        );
    }

    #[test]
    fn test_case_option_v2() {
        let matcher = SkimMatcherV2::default().ignore_case();
        assert!(matcher.fuzzy_match("aBc", "abc").is_some());
        assert!(matcher.fuzzy_match("aBc", "aBc").is_some());
        assert!(matcher.fuzzy_match("aBc", "aBC").is_some());

        let matcher = SkimMatcherV2::default().respect_case();
        assert!(matcher.fuzzy_match("aBc", "abc").is_none());
        assert!(matcher.fuzzy_match("aBc", "aBc").is_some());
        assert!(matcher.fuzzy_match("aBc", "aBC").is_none());

        let matcher = SkimMatcherV2::default().smart_case();
        assert!(matcher.fuzzy_match("aBc", "abc").is_some());
        assert!(matcher.fuzzy_match("aBc", "aBc").is_some());
        assert!(matcher.fuzzy_match("aBc", "aBC").is_none());
    }

    #[test]
    fn test_matcher_quality_v2() {
        let matcher = SkimMatcherV2::default();
        assert_order(&matcher, "ab", &["ab", "aoo_boo", "acb"]);
        assert_order(
            &matcher,
            "cc",
            &[
                "camel case",
                "camelCase",
                "CamelCase",
                "camelcase",
                "camel ace",
            ],
        );
        assert_order(
            &matcher,
            "Da.Te",
            &["Data.Text", "Data.Text.Lazy", "Data.Aeson.Encoding.Text"],
        );
        assert_order(&matcher, "is", &["isIEEE", "inSuf"]);
        assert_order(&matcher, "ma", &["map", "many", "maximum"]);
        assert_order(&matcher, "print", &["printf", "sprintf"]);
        assert_order(&matcher, "ast", &["ast", "AST", "INT_FAST16_MAX"]);
        assert_order(&matcher, "int", &["int", "INT", "PRINT"]);
    }
}
