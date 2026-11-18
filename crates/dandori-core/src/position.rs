//! 並び順のキー（fractional indexing）。
//!
//! キーは `0-9A-Za-z` の62進数の小数部分とみなし、辞書順で比較する。
//! 2つのキーの間に新しいキーを作れるので、移動しても変わるのは対象の1行だけで済む。
//! キーは末尾が `0` にならないように作る（`"V"` と `"V0"` が同値になるのを避けるため）。

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

fn digit_index(c: u8) -> usize {
    DIGITS
        .iter()
        .position(|&d| d == c)
        .unwrap_or_else(|| panic!("invalid position digit: {}", c as char))
}

/// `a < key < b` となるキーを返す。`a` が空文字なら下限なし、`b` が `None` なら上限なし。
///
/// # Panics
/// `b` が指定されていて `a >= b` の場合
pub fn key_between(a: &str, b: Option<&str>) -> String {
    if let Some(b) = b {
        assert!(a < b, "key_between: {a:?} must be less than {b:?}");
    }
    midpoint(a.as_bytes(), b.map(str::as_bytes))
}

fn midpoint(a: &[u8], b: Option<&[u8]>) -> String {
    if let Some(b) = b {
        // 共通の接頭辞を取り除く（a は右側を '0' で埋めたものとして扱う）
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(b'0') == b[n] {
            n += 1;
        }
        if n > 0 {
            let rest_a = a.get(n..).unwrap_or(&[]);
            let mut s = String::from_utf8(b[..n].to_vec()).expect("ascii");
            s.push_str(&midpoint(rest_a, Some(&b[n..])));
            return s;
        }
    }
    let da = a.first().map_or(0, |&c| digit_index(c));
    let db = b.map_or(BASE, |b| digit_index(b[0]));
    if db - da > 1 {
        let mid = (da + db).div_ceil(2);
        (DIGITS[mid] as char).to_string()
    } else if let Some(b) = b.filter(|b| b.len() > 1) {
        (b[0] as char).to_string()
    } else {
        let mut s = (DIGITS[da] as char).to_string();
        s.push_str(&midpoint(a.get(1..).unwrap_or(&[]), None));
        s
    }
}

/// `n` 個の、等間隔で昇順のキーを作る（キーが重複したときの振り直しに使う）
pub fn even_keys(n: usize) -> Vec<String> {
    let base = BASE as u128;
    let slots = n as u128 + 1;
    let mut width = 1u32;
    while base.pow(width) / slots < base {
        width += 1;
    }
    let step = base.pow(width) / slots;
    (1..=n as u128)
        .map(|i| {
            let mut v = step * i;
            let mut digits = vec![b'0'; width as usize];
            for d in digits.iter_mut().rev() {
                *d = DIGITS[(v % base) as usize];
                v /= base;
            }
            // 末尾の '0' を避ける（step >= 62 なので順序は崩れない）
            if let Some(last) = digits.last_mut().filter(|d| **d == b'0') {
                *last = b'1';
            }
            String::from_utf8(digits).expect("ascii")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_valid(k: &str) {
        assert!(!k.is_empty());
        assert!(!k.ends_with('0'), "trailing zero: {k}");
    }

    #[test]
    fn first_key_is_middle() {
        assert_eq!(key_between("", None), "V");
    }

    #[test]
    fn keys_between_are_ordered() {
        let cases = [
            ("", Some("V")),
            ("V", None),
            ("V", Some("W")),
            ("a", Some("a1")),
            ("0V", Some("1")),
            ("z", None),
        ];
        for (a, b) in cases {
            let k = key_between(a, b);
            assert_valid(&k);
            assert!(a < k.as_str(), "{a} < {k}");
            if let Some(b) = b {
                assert!(k.as_str() < b, "{k} < {b}");
            }
        }
    }

    #[test]
    fn repeated_insert_at_top_and_bottom() {
        let mut keys = vec![key_between("", None)];
        for _ in 0..500 {
            let top = key_between("", Some(&keys[0]));
            keys.insert(0, top);
            let bottom = key_between(keys.last().unwrap(), None);
            keys.push(bottom);
        }
        for w in keys.windows(2) {
            assert!(w[0] < w[1]);
            assert_valid(&w[0]);
        }
    }

    #[test]
    fn repeated_insert_in_middle() {
        let mut lo = key_between("", None);
        let hi = key_between(&lo, None);
        for _ in 0..200 {
            let k = key_between(&lo, Some(&hi));
            assert!(lo < k && k < hi);
            assert_valid(&k);
            lo = k;
        }
    }

    #[test]
    fn even_keys_are_sorted_and_valid() {
        for n in [0, 1, 2, 10, 61, 62, 63, 1000] {
            let keys = even_keys(n);
            assert_eq!(keys.len(), n);
            for w in keys.windows(2) {
                assert!(w[0] < w[1]);
            }
            keys.iter().for_each(|k| assert_valid(k));
            if n >= 2 {
                // 振り直した後も間に挿入できること
                let k = key_between(&keys[0], Some(&keys[1]));
                assert!(keys[0] < k && k < keys[1]);
            }
        }
    }
}
