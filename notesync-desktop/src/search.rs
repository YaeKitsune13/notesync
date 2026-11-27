use std::ops::Range;

// Все вхождения query в text без учёта регистра. Возвращает байтовые диапазоны text.
pub fn find_all(text: &str, query: &str) -> Vec<Range<usize>> {
    let needle: Vec<char> = query
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        let hit = (0..needle.len()).all(|k| {
            chars[i + k]
                .1
                .to_lowercase()
                .next()
                .unwrap_or(chars[i + k].1)
                == needle[k]
        });
        if hit {
            let start = chars[i].0;
            let end = chars
                .get(i + needle.len())
                .map(|c| c.0)
                .unwrap_or(text.len());
            out.push(start..end);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}
