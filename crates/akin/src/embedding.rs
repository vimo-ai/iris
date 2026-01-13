use ndarray::Array1;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EmbeddingError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {0}")]
    Api(String),
}

pub type Result<T> = std::result::Result<T, EmbeddingError>;

/// Ollama 嵌入生成器
pub struct OllamaEmbedding {
    client: Option<Client>,
    base_url: String,
    model: String,
}

#[derive(Serialize)]
struct EmbedRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbedding {
    pub fn new(model: &str) -> Self {
        Self {
            client: None, // Lazy init
            base_url: "http://localhost:11434".to_string(),
            model: model.to_string(),
        }
    }

    /// 获取或创建 HTTP client
    fn get_client(&mut self) -> Result<&Client> {
        if self.client.is_none() {
            let client = Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| EmbeddingError::Http(e))?;
            self.client = Some(client);
        }
        Ok(self.client.as_ref().unwrap())
    }

    pub fn with_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    /// 生成单个文本的嵌入
    pub async fn embed(&mut self, text: &str) -> Result<Array1<f32>> {
        // Clone values before mutable borrow
        let url = format!("{}/api/embed", self.base_url);
        let request = EmbedRequest {
            model: self.model.clone(),
            input: text.to_string(),
        };

        let client = self.get_client()?;
        let response = client
            .post(url)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(EmbeddingError::Api(format!(
                "Ollama returned status {}",
                response.status()
            )));
        }

        let data: EmbedResponse = response.json().await?;
        let embedding = data
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::Api("No embedding returned".into()))?;

        Ok(Array1::from_vec(embedding))
    }

    /// 批量生成嵌入
    pub async fn embed_batch(&mut self, texts: &[&str]) -> Result<Vec<Array1<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }
}

/// 余弦相似度
pub fn cosine_similarity(a: &Array1<f32>, b: &Array1<f32>) -> f32 {
    let dot = a.dot(b);
    let norm_a = a.dot(a).sqrt();
    let norm_b = b.dot(b).sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// 嵌入转字节 (用于数据库存储)
pub fn embedding_to_bytes(embedding: &Array1<f32>) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect()
}

/// 字节转嵌入 (返回 None 如果字节数不是 4 的倍数)
pub fn bytes_to_embedding(bytes: &[u8]) -> Option<Array1<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    let floats: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| {
            // Safety: chunks_exact(4) guarantees exactly 4 bytes
            f32::from_le_bytes(chunk.try_into().unwrap())
        })
        .collect();
    Some(Array1::from_vec(floats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let a = array![1.0, 2.0, 3.0];
        let b = array![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let a = array![1.0, 0.0, 0.0];
        let b = array![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let a = array![1.0, 2.0, 3.0];
        let b = array![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = array![0.0, 0.0, 0.0];
        let b = array![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_cosine_similarity_similar_vectors() {
        let a = array![1.0, 2.0, 3.0];
        let b = array![1.1, 2.1, 3.1];
        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.99); // Very similar
    }

    #[test]
    fn test_embedding_to_bytes_roundtrip() {
        let original = array![1.0_f32, 2.5, -3.14, 0.0];
        let bytes = embedding_to_bytes(&original);
        let recovered = bytes_to_embedding(&bytes).unwrap();

        assert_eq!(original.len(), recovered.len());
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn test_embedding_to_bytes_length() {
        let emb = array![1.0_f32, 2.0, 3.0];
        let bytes = embedding_to_bytes(&emb);
        assert_eq!(bytes.len(), 12); // 3 floats * 4 bytes
    }

    #[test]
    fn test_bytes_to_embedding_invalid_length() {
        let bytes = vec![1, 2, 3]; // Not divisible by 4
        assert!(bytes_to_embedding(&bytes).is_none());
    }

    #[test]
    fn test_bytes_to_embedding_empty() {
        let bytes: Vec<u8> = vec![];
        let result = bytes_to_embedding(&bytes);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_ollama_embedding_builder() {
        let emb = OllamaEmbedding::new("bge-m3")
            .with_url("http://custom:11434");
        assert_eq!(emb.base_url, "http://custom:11434");
        assert_eq!(emb.model, "bge-m3");
    }

    /// 端到端测试：验证属性上下文对相似度检测的影响
    ///
    /// 测试场景：
    /// 1. 真正重复: 两个类中结构和语义都相似的方法 (应该高相似度)
    /// 2. 假阳性: 方法结构相似但类属性完全不同 (应该能区分)
    ///
    /// 需要本地运行 Ollama 服务
    #[tokio::test]
    #[ignore] // 默认跳过，需要 Ollama 服务
    async fn test_property_context_improves_similarity_detection() {
        let mut emb = OllamaEmbedding::new("bge-m3");

        // === 真正的重复: 两个 Session 类的 establish 方法 ===
        // 语义相同，应该被检测为重复

        let session_a = r#"
// Class properties:
var sessionId: String = ""
var userId: String = ""
private var createdAt: Date = Date()

func establish(userId: String) {
    self.sessionId = UUID().uuidString
    self.userId = userId
    self.createdAt = Date()
}
"#;

        let session_b = r#"
// Class properties:
var sessionToken: String = ""
var currentUserId: String = ""
private var startTime: Date = Date()

func establish(userId: String) {
    self.sessionToken = UUID().uuidString
    self.currentUserId = userId
    self.startTime = Date()
}
"#;

        // === 假阳性: 方法结构相似但语义完全不同 ===
        // 文件管理 vs 会话管理，不应该被认为是重复

        let file_manager = r#"
// Class properties:
var filePath: String = ""
var fileSize: Int = 0
private var modifiedAt: Date = Date()

func establish(path: String) {
    self.filePath = path
    self.fileSize = 0
    self.modifiedAt = Date()
}
"#;

        // === 纯函数对比 (无属性上下文) ===
        let func_only_session = r#"
func establish(userId: String) {
    self.sessionId = UUID().uuidString
    self.userId = userId
    self.createdAt = Date()
}
"#;

        let func_only_file = r#"
func establish(path: String) {
    self.filePath = path
    self.fileSize = 0
    self.modifiedAt = Date()
}
"#;

        // 生成 embeddings
        let emb_session_a = emb.embed(session_a).await.expect("embed failed");
        let emb_session_b = emb.embed(session_b).await.expect("embed failed");
        let emb_file_manager = emb.embed(file_manager).await.expect("embed failed");
        let emb_func_session = emb.embed(func_only_session).await.expect("embed failed");
        let emb_func_file = emb.embed(func_only_file).await.expect("embed failed");

        // 计算相似度
        let sim_real_duplicate = cosine_similarity(&emb_session_a, &emb_session_b);
        let sim_with_context = cosine_similarity(&emb_session_a, &emb_file_manager);
        let sim_without_context = cosine_similarity(&emb_func_session, &emb_func_file);

        println!("\n=== 属性上下文对相似度检测的影响 ===\n");

        println!("1. 真正的重复 (SessionA vs SessionB):");
        println!("   相似度: {:.2}%", sim_real_duplicate * 100.0);
        println!("   预期: 高相似度 (两个会话管理类)\n");

        println!("2. 有属性上下文 (Session vs FileManager):");
        println!("   相似度: {:.2}%", sim_with_context * 100.0);
        println!("   属性: sessionId/userId vs filePath/fileSize\n");

        println!("3. 无属性上下文 (纯函数对比):");
        println!("   相似度: {:.2}%", sim_without_context * 100.0);
        println!("   只比较 establish() 方法体\n");

        // 更直接的对比: 同一假阳性场景，有/无属性上下文
        let session_no_ctx = r#"
func establish(userId: String) {
    self.sessionId = UUID().uuidString
    self.userId = userId
    self.createdAt = Date()
}
"#;
        let file_no_ctx = r#"
func establish(path: String) {
    self.filePath = path
    self.fileSize = 0
    self.modifiedAt = Date()
}
"#;
        let emb_session_no_ctx = emb.embed(session_no_ctx).await.expect("embed failed");
        let emb_file_no_ctx = emb.embed(file_no_ctx).await.expect("embed failed");
        let sim_fp_no_ctx = cosine_similarity(&emb_session_no_ctx, &emb_file_no_ctx);

        // Session (有上下文) vs File (有上下文)
        let sim_fp_with_ctx = sim_with_context;

        println!("=== 假阳性检测对比 (Session vs FileManager) ===");
        println!("无属性上下文: {:.2}%", sim_fp_no_ctx * 100.0);
        println!("有属性上下文: {:.2}%", sim_fp_with_ctx * 100.0);

        println!("\n=== 结论 (阈值 85%) ===");
        println!("真正重复 (SessionA vs SessionB): {:.2}% → {}", sim_real_duplicate * 100.0,
            if sim_real_duplicate >= 0.85 { "✅ 检测为重复" } else { "❌ 漏检" });
        println!("假阳性 (Session vs FileManager): {:.2}% → {}", sim_fp_with_ctx * 100.0,
            if sim_fp_with_ctx < 0.85 { "✅ 正确排除" } else { "❌ 误报" });
    }
}
