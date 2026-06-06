pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

pub fn cosine_similarity_768(a: &[f32; 768], b: &[f32; 768]) -> f32 {
    cosine_similarity(a, b)
}

pub fn find_top_k(query: &[f32; 768], entries: &[[f32; 768]], k: usize) -> Vec<(usize, f32)> {
    let mut scores: Vec<(usize, f32)> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| (i, cosine_similarity_768(query, entry)))
        .collect();

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(k);
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = [1.0f32; 768];
        let b = [1.0f32; 768];
        let sim = cosine_similarity_768(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let mut a = [0.0f32; 768];
        let mut b = [0.0f32; 768];
        a[0] = 1.0;
        b[1] = 1.0;
        let sim = cosine_similarity_768(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = [1.0f32; 768];
        let b = [-1.0f32; 768];
        let sim = cosine_similarity_768(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_find_top_k() {
        let mut entries = vec![[0.0f32; 768]; 5];
        entries[0][0] = 1.0;
        entries[1][0] = 0.8;
        entries[2][0] = 0.6;
        entries[3][0] = 0.4;
        entries[4][0] = 0.2;

        let mut query = [0.0f32; 768];
        query[0] = 1.0;
        let results = find_top_k(&query, &entries, 3);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, 0);
        assert_eq!(results[1].0, 1);
        assert_eq!(results[2].0, 2);
    }
}
