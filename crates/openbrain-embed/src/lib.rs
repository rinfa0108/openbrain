pub trait EmbeddingProvider {
    fn embed(&self, text: &str) -> Vec<f32>;
}
