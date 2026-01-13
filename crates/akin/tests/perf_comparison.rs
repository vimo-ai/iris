//! 性能对比测试：ANN vs 暴力搜索

use std::time::Instant;
use akin::{VectorIndex, VectorIndexConfig};

/// 生成随机向量
fn random_vector(dim: usize, seed: u64) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    let base = hasher.finish();

    (0..dim)
        .map(|i| {
            let mut h = DefaultHasher::new();
            (base + i as u64).hash(&mut h);
            let v = h.finish();
            ((v % 10000) as f32 / 10000.0) * 2.0 - 1.0
        })
        .collect()
}

/// 暴力搜索
fn brute_force_search(
    vectors: &[Vec<f32>],
    query: &[f32],
    k: usize,
) -> Vec<(usize, f32)> {
    let mut distances: Vec<(usize, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let dot: f32 = query.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
            let norm_q: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm_v: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            let cos_sim = dot / (norm_q * norm_v + 1e-10);
            let cos_dist = 1.0 - cos_sim;
            (i, cos_dist)
        })
        .collect();

    distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    distances.truncate(k);
    distances
}

#[test]
fn test_perf_comparison() {
    let dim = 1024;
    let n_vectors = 5000; // 模拟 5000 个代码单元
    let k = 50;
    let n_queries = 10;

    println!("\n=== 性能对比测试 ===");
    println!("向量数量: {}", n_vectors);
    println!("向量维度: {}", dim);
    println!("查询数量: {}", n_queries);
    println!("Top-K: {}", k);
    println!();

    // 生成测试数据
    let vectors: Vec<Vec<f32>> = (0..n_vectors)
        .map(|i| random_vector(dim, i as u64))
        .collect();

    let queries: Vec<Vec<f32>> = (0..n_queries)
        .map(|i| random_vector(dim, (n_vectors + i) as u64))
        .collect();

    // 1. 暴力搜索基准
    let start = Instant::now();
    for query in &queries {
        let _ = brute_force_search(&vectors, query, k);
    }
    let brute_force_time = start.elapsed();
    let brute_force_per_query = brute_force_time.as_micros() as f64 / n_queries as f64;

    println!("暴力搜索:");
    println!("  总耗时: {:?}", brute_force_time);
    println!("  每次查询: {:.2} µs", brute_force_per_query);

    // 2. ANN 搜索
    let config = VectorIndexConfig {
        dimensions: dim,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 64,
    };
    let index = VectorIndex::new(config).unwrap();
    index.reserve(n_vectors + 100).unwrap();

    // 建索引
    let start = Instant::now();
    for (i, vec) in vectors.iter().enumerate() {
        index.add(i as u64, vec).unwrap();
    }
    let index_time = start.elapsed();

    // 搜索
    let start = Instant::now();
    for query in &queries {
        let _ = index.search(query, k).unwrap();
    }
    let ann_time = start.elapsed();
    let ann_per_query = ann_time.as_micros() as f64 / n_queries as f64;

    println!("\nANN (HNSW) 搜索:");
    println!("  建索引耗时: {:?}", index_time);
    println!("  搜索总耗时: {:?}", ann_time);
    println!("  每次查询: {:.2} µs", ann_per_query);

    // 3. 对比
    let speedup = brute_force_per_query / ann_per_query;
    println!("\n=== 结果 ===");
    println!("加速比: {:.1}x", speedup);
    println!(
        "暴力搜索 {:.2} ms/query -> ANN {:.2} ms/query",
        brute_force_per_query / 1000.0,
        ann_per_query / 1000.0
    );

    // 验证 ANN 比暴力搜索快（应该快 10x 以上）
    assert!(
        speedup > 5.0,
        "ANN should be at least 5x faster than brute force"
    );
}

#[test]
fn test_ann_recall() {
    // 测试 ANN 的召回率
    let dim = 128; // 用小维度快速测试
    let n_vectors = 1000;
    let k = 10;

    let vectors: Vec<Vec<f32>> = (0..n_vectors)
        .map(|i| random_vector(dim, i as u64))
        .collect();

    let query = random_vector(dim, 99999);

    // 暴力搜索获取真实 top-k
    let ground_truth: Vec<usize> = brute_force_search(&vectors, &query, k)
        .iter()
        .map(|(i, _)| *i)
        .collect();

    // ANN 搜索
    let config = VectorIndexConfig {
        dimensions: dim,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 64,
    };
    let index = VectorIndex::new(config).unwrap();
    index.reserve(n_vectors + 100).unwrap();

    for (i, vec) in vectors.iter().enumerate() {
        index.add(i as u64, vec).unwrap();
    }

    let ann_results = index.search(&query, k).unwrap();
    let ann_ids: Vec<usize> = ann_results.iter().map(|r| r.id as usize).collect();

    // 计算召回率
    let hits: usize = ann_ids
        .iter()
        .filter(|id| ground_truth.contains(id))
        .count();
    let recall = hits as f64 / k as f64;

    println!("\n=== 召回率测试 ===");
    println!("Ground truth: {:?}", ground_truth);
    println!("ANN results:  {:?}", ann_ids);
    println!("召回率: {:.1}%", recall * 100.0);

    // 召回率应该至少 80%
    assert!(recall >= 0.8, "Recall should be at least 80%");
}
