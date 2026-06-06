pub struct L1ValueClassifier {
    keywords: Vec<String>,
}

impl L1ValueClassifier {
    pub fn new(keywords: Vec<String>) -> Self {
        Self { keywords }
    }

    pub fn classify(&self, text: &str) -> ValueAssessment {
        let text_lower = text.to_lowercase();
        let mut jargon_count = 0;

        for keyword in &self.keywords {
            if text_lower.contains(&keyword.to_lowercase()) {
                jargon_count += 1;
            }
        }

        let is_jargon = jargon_count > 2;
        let probability = if is_jargon { 0.3 } else { 0.9 };

        ValueAssessment {
            probability,
            is_jargon,
            jargon_count,
        }
    }
}

pub struct ValueAssessment {
    pub probability: f32,
    pub is_jargon: bool,
    pub jargon_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_classifier() {
        let classifier = L1ValueClassifier::new(vec![
            "urgent".to_string(),
            "critical".to_string(),
            "emergency".to_string(),
        ]);

        let assessment = classifier.classify("This is an urgent critical emergency");
        assert!(assessment.is_jargon);

        let assessment = classifier.classify("This is a normal task");
        assert!(!assessment.is_jargon);
    }
}
