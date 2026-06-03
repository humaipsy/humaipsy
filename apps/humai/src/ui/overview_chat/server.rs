use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use std::sync::mpsc;

use super::audit::{self, AuditRecord, SCHEMA_VERSION};
use super::chat::{ChatMessage, MessageCorrelation};
use super::incoming::MessageSource;
use super::ollama::{self, OllamaChatOptions, OllamaMessage};
