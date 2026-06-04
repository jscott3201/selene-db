//! Local-only oMLX embedding rows for realistic vector distributions.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{
    CancellationChecker, GraphId, HnswIndexConfig, LabelSet, NodeId, PropertyMap, Value,
    VectorMetric, VectorValue,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SeleneGraph, SharedGraph, VectorIndexConfig, VectorIndexKind,
};

use crate::common::scale_label;

use super::support::istr;

const ENABLE_ENV: &str = "SELENE_OMLX_EMBEDDING_BENCH";
const API_KEY_ENVS: &[&str] = &["SELENE_OMLX_API_KEY", "OMLX_KEY"];
const BASE_URL_ENV: &str = "SELENE_OMLX_BASE_URL";
const MODELS_ENV: &str = "SELENE_OMLX_EMBEDDING_MODELS";
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:7700/v1";
const DEFAULT_MODELS: &[&str] = &[
    "Qwen3-Embedding-0.6B-4bit-DWQ",
    "Qwen3-Embedding-4B-4bit-DWQ",
];
const TOP_K: usize = 4;
const ANN_SEARCH_WIDTH: usize = 64;

pub(super) fn bench(c: &mut Criterion) {
    let Some(config) = OmlxBenchConfig::from_env() else {
        return;
    };
    let client = OmlxClient::new(config.base_url, config.api_key);
    let inputs = corpus_inputs();
    let mut group = c.benchmark_group("graph_vector_omlx_embedding_pressure");
    for model in config.models {
        let model_id = model_id(&model);
        let vectors = client
            .embed(&model, &inputs)
            .expect("local oMLX embedding request succeeds");
        let fixture = OmlxVectorFixture::build(&model, vectors);
        group.throughput(Throughput::Elements(inputs.len() as u64));
        group.bench_function(
            BenchmarkId::new(
                "embed_batch",
                format!("{}_docs{}_dim{}", model_id, inputs.len(), fixture.dimension),
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        client
                            .embed(&model, &inputs)
                            .expect("local oMLX embedding request succeeds"),
                    );
                });
            },
        );
        group.throughput(Throughput::Elements((fixture.query_count() * TOP_K) as u64));
        group.bench_function(
            BenchmarkId::new(
                "exact_graph_search",
                format!(
                    "{}_{}_q{}_k{}_dim{}_precbp{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    fixture.query_count(),
                    TOP_K,
                    fixture.dimension,
                    fixture.exact_precision_basis_points(),
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.exact_total_precision()));
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "hnsw_graph_search",
                format!(
                    "{}_{}_q{}_k{}_ef{}_dim{}_precbp{}",
                    model_id,
                    scale_label(fixture.document_count()),
                    fixture.query_count(),
                    TOP_K,
                    ANN_SEARCH_WIDTH,
                    fixture.dimension,
                    fixture.ann_precision_basis_points(),
                ),
            ),
            |b| {
                b.iter(|| black_box(fixture.ann_total_precision()));
            },
        );
    }
    group.finish();
}

struct OmlxBenchConfig {
    base_url: String,
    api_key: String,
    models: Vec<String>,
}

impl OmlxBenchConfig {
    fn from_env() -> Option<Self> {
        if std::env::var(ENABLE_ENV).ok().as_deref() != Some("1") {
            return None;
        }
        let api_key = API_KEY_ENVS
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .expect("SELENE_OMLX_API_KEY or OMLX_KEY must be set for local oMLX benches");
        let base_url = std::env::var(BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
        let models = std::env::var(MODELS_ENV)
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|models| !models.is_empty())
            .unwrap_or_else(|| {
                DEFAULT_MODELS
                    .iter()
                    .map(|model| (*model).to_owned())
                    .collect()
            });
        Some(Self {
            base_url,
            api_key,
            models,
        })
    }
}

struct OmlxClient {
    endpoint: HttpEndpoint,
    api_key: String,
}

impl OmlxClient {
    fn new(base_url: String, api_key: String) -> Self {
        let endpoint = HttpEndpoint::parse(&base_url).expect("valid local oMLX HTTP base URL");
        Self { endpoint, api_key }
    }

    fn embed(&self, model: &str, inputs: &[CorpusInput]) -> Result<Vec<VectorValue>, String> {
        let body = serde_json::json!({
            "model": model,
            "input": inputs.iter().map(|input| input.text).collect::<Vec<_>>(),
        })
        .to_string();
        let response = self
            .endpoint
            .post_json("/embeddings", &self.api_key, &body)?;
        let json: serde_json::Value = serde_json::from_slice(&response)
            .map_err(|err| format!("oMLX embedding response is not JSON: {err}"))?;
        let Some(data) = json.get("data").and_then(|value| value.as_array()) else {
            return Err(format!(
                "oMLX embedding response for {model} has no data array: {}",
                truncate_json(&json)
            ));
        };
        data.iter()
            .map(|item| {
                let Some(values) = item.get("embedding").and_then(|value| value.as_array()) else {
                    return Err("oMLX embedding item has no embedding array".to_owned());
                };
                let components = values
                    .iter()
                    .map(|value| {
                        value
                            .as_f64()
                            .map(|component| component as f32)
                            .ok_or_else(|| "oMLX embedding component is not numeric".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                VectorValue::new(components)
                    .map_err(|err| format!("oMLX embedding vector failed validation: {err}"))
            })
            .collect()
    }
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path_prefix: String,
}

impl HttpEndpoint {
    fn parse(base_url: &str) -> Result<Self, String> {
        let Some(rest) = base_url.strip_prefix("http://") else {
            return Err("local oMLX bench only supports http:// endpoints".to_owned());
        };
        let (host_port, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port) = match host_port.rsplit_once(':') {
            Some((host, port)) => {
                let port = port
                    .parse::<u16>()
                    .map_err(|err| format!("invalid oMLX port: {err}"))?;
                (host.to_owned(), port)
            }
            None => (host_port.to_owned(), 80),
        };
        if host.is_empty() {
            return Err("local oMLX host must not be empty".to_owned());
        }
        let path_prefix = if path.is_empty() {
            String::new()
        } else {
            format!("/{}", path.trim_end_matches('/'))
        };
        Ok(Self {
            host,
            port,
            path_prefix,
        })
    }

    fn post_json(&self, suffix: &str, api_key: &str, body: &str) -> Result<Vec<u8>, String> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .map_err(|err| format!("connect to local oMLX failed: {err}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(120)))
            .map_err(|err| format!("set local oMLX read timeout failed: {err}"))?;
        let path = format!("{}{}", self.path_prefix, suffix);
        let request = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: {host}:{port}\r\n\
             Authorization: Bearer {api_key}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {len}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            host = self.host,
            port = self.port,
            len = body.len(),
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|err| format!("write local oMLX request failed: {err}"))?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|err| format!("read local oMLX response failed: {err}"))?;
        parse_http_body(&response)
    }
}

fn parse_http_body(response: &[u8]) -> Result<Vec<u8>, String> {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err("local oMLX response missing HTTP header terminator".to_owned());
    };
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|err| format!("local oMLX response header is not UTF-8: {err}"))?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "local oMLX response missing HTTP status".to_owned())?;
    let mut body = response[header_end + 4..].to_vec();
    if has_chunked_transfer_encoding(headers) {
        body = decode_chunked_body(&body)?;
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "local oMLX request failed with HTTP {status}: {}",
            String::from_utf8_lossy(&body)
                .chars()
                .take(240)
                .collect::<String>()
        ));
    }
    Ok(body)
}

fn has_chunked_transfer_encoding(headers: &str) -> bool {
    headers.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("transfer-encoding:") && lower.contains("chunked")
    })
}

fn decode_chunked_body(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    let mut offset = 0usize;
    loop {
        let line_end = find_crlf(raw, offset)
            .ok_or_else(|| "chunked oMLX response missing chunk-size line end".to_owned())?;
        let size_line = std::str::from_utf8(&raw[offset..line_end])
            .map_err(|err| format!("chunked oMLX size line is not UTF-8: {err}"))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|err| format!("chunked oMLX size is not hex: {err}"))?;
        offset = line_end + 2;
        if size == 0 {
            break;
        }
        let chunk_end = offset
            .checked_add(size)
            .ok_or_else(|| "chunked oMLX body size overflow".to_owned())?;
        if chunk_end + 2 > raw.len() {
            return Err("chunked oMLX body ended before declared chunk size".to_owned());
        }
        decoded.extend_from_slice(&raw[offset..chunk_end]);
        if &raw[chunk_end..chunk_end + 2] != b"\r\n" {
            return Err("chunked oMLX chunk missing trailing CRLF".to_owned());
        }
        offset = chunk_end + 2;
    }
    Ok(decoded)
}

fn find_crlf(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|position| start + position)
}

struct OmlxVectorFixture {
    graph: SeleneGraph,
    label: selene_core::IStr,
    embedding_key: selene_core::IStr,
    dimension: usize,
    documents: Vec<DocumentMeta>,
    topics_by_node: HashMap<NodeId, Topic>,
    queries: Vec<QueryVector>,
}

impl OmlxVectorFixture {
    fn build(model: &str, vectors: Vec<VectorValue>) -> Self {
        let inputs = corpus_inputs();
        assert_eq!(
            vectors.len(),
            inputs.len(),
            "oMLX returned one vector per corpus input"
        );
        let dimension = vectors
            .first()
            .map(VectorValue::dimension)
            .expect("corpus has at least one vector");
        assert!(
            vectors.iter().all(|vector| vector.dimension() == dimension),
            "oMLX returned consistent vector dimensions"
        );
        let label = istr("OmlxEmbeddingDoc");
        let embedding_key = istr("embedding");
        let shared = SharedGraph::new(graph_id_for_model(model));
        let mut documents = Vec::new();
        {
            let mut txn = shared.begin_write();
            {
                let mut mutator = txn.mutator();
                for (input, vector) in inputs.iter().zip(vectors.iter()) {
                    if !input.is_document {
                        continue;
                    }
                    let props = PropertyMap::from_pairs([(
                        embedding_key.clone(),
                        Value::Vector(vector.clone()),
                    )])
                    .expect("oMLX bench document properties fit");
                    let node = mutator
                        .create_node(LabelSet::single(label.clone()), props)
                        .expect("oMLX bench document node inserts");
                    documents.push(DocumentMeta {
                        node,
                        topic: input.topic,
                    });
                }
                mutator
                    .create_vector_index_named_with_configs(
                        label.clone(),
                        embedding_key.clone(),
                        VectorIndexKind::HnswCosine,
                        dimension as u32,
                        None,
                        VectorIndexConfig::new(Some(HnswIndexConfig::new(16, 64)), None),
                    )
                    .expect("oMLX bench HNSW index builds");
            }
            txn.commit().expect("oMLX bench graph commits");
        }
        let queries = inputs
            .into_iter()
            .zip(vectors)
            .filter_map(|(input, vector)| {
                (!input.is_document).then_some(QueryVector {
                    topic: input.topic,
                    vector,
                })
            })
            .collect();
        let topics_by_node = documents
            .iter()
            .map(|document| (document.node, document.topic))
            .collect();
        Self {
            graph: shared.read().as_ref().clone(),
            label,
            embedding_key,
            dimension,
            documents,
            topics_by_node,
            queries,
        }
    }

    fn document_count(&self) -> usize {
        self.documents.len()
    }

    fn query_count(&self) -> usize {
        self.queries.len()
    }

    fn exact_total_precision(&self) -> usize {
        self.queries
            .iter()
            .map(|query| {
                let hits = self
                    .graph
                    .exact_vector_search_nodes_checked(
                        &self.label,
                        &self.embedding_key,
                        &query.vector,
                        VectorMetric::Cosine,
                        TOP_K,
                        CancellationChecker::disabled(),
                    )
                    .expect("oMLX exact vector search succeeds");
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
    }

    fn ann_total_precision(&self) -> usize {
        self.queries
            .iter()
            .map(|query| {
                let hits = self
                    .graph
                    .approximate_vector_search_nodes_checked(
                        &self.label,
                        &self.embedding_key,
                        &query.vector,
                        ApproximateVectorSearchOptions::new(
                            VectorMetric::Cosine,
                            TOP_K,
                            ANN_SEARCH_WIDTH,
                        ),
                        CancellationChecker::disabled(),
                    )
                    .expect("oMLX ANN vector search succeeds");
                self.precision(query.topic, hits.into_iter().map(|hit| hit.node_id))
            })
            .sum()
    }

    fn exact_precision_basis_points(&self) -> usize {
        basis_points(self.exact_total_precision(), self.query_count() * TOP_K)
    }

    fn ann_precision_basis_points(&self) -> usize {
        basis_points(self.ann_total_precision(), self.query_count() * TOP_K)
    }

    fn precision<I>(&self, topic: Topic, hits: I) -> usize
    where
        I: IntoIterator<Item = NodeId>,
    {
        hits.into_iter()
            .filter(|node| {
                self.topics_by_node
                    .get(node)
                    .is_some_and(|hit_topic| *hit_topic == topic)
            })
            .count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Topic {
    Gql,
    Vector,
    AgentMemory,
    Code,
}

#[derive(Clone, Copy)]
struct CorpusInput {
    topic: Topic,
    is_document: bool,
    text: &'static str,
}

struct DocumentMeta {
    node: NodeId,
    topic: Topic,
}

struct QueryVector {
    topic: Topic,
    vector: VectorValue,
}

fn corpus_inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    for (topic, texts) in [
        (
            Topic::Gql,
            &[
                "ISO GQL MATCH over graph patterns returns rows from a property graph.",
                "A GQL CALL procedure can expose implementation-defined graph algorithms.",
                "Graph type constraints validate labels and property value types.",
                "Serializable graph transactions preserve committed mutation ordering.",
            ][..],
        ),
        (
            Topic::Vector,
            &[
                "A dense embedding vector is stored as a first-class graph property value.",
                "HNSW approximate nearest-neighbor search trades recall for latency.",
                "IVF partitions vectors through coarse centroids before reranking candidates.",
                "Candidate-set scoring exact-reranks graph-derived vector candidates.",
            ][..],
        ),
        (
            Topic::AgentMemory,
            &[
                "Agent memory retrieval should prefer current facts over superseded evidence.",
                "Contradiction edges mark unresolved memory facts that should be filtered.",
                "Session and scope nodes narrow memory retrieval to a task-local subgraph.",
                "Recency windows and dependency edges produce active memory candidate sets.",
            ][..],
        ),
        (
            Topic::Code,
            &[
                "Rust benchmarks should use Criterion groups with stable benchmark IDs.",
                "A VectorCandidateSet stores sorted and deduplicated NodeId values.",
                "The score_vector_candidate_sets_batch_checked API scores many queries.",
                "A safe local HTTP client can post JSON without adding a runtime dependency.",
            ][..],
        ),
    ] {
        inputs.extend(texts.iter().map(|text| CorpusInput {
            topic,
            is_document: true,
            text,
        }));
    }
    inputs.extend([
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "How does GQL execute graph pattern matching and procedure calls?",
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which vector index should rerank embedding candidates in memory?",
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Find current task memory while ignoring contradicted facts.",
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Where is the Rust batch vector candidate scoring API implemented?",
        },
    ]);
    inputs
}

fn graph_id_for_model(model: &str) -> GraphId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    GraphId::new(97_000 + hasher.finish() % 1_000)
}

fn model_id(model: &str) -> String {
    model
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}

fn truncate_json(json: &serde_json::Value) -> String {
    json.to_string().chars().take(240).collect()
}
