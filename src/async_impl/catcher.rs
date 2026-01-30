use std::{
    mem, sync::{Mutex, mpsc::{self, Receiver, Sender}}, thread::{self, JoinHandle}
};

use anyhow::bail;
use http::HeaderMap;
use crate::{Request, Response, SerializableResponse};
use rusqlite::Connection;
use serde_json::Value;

enum CatcherRequest {
    /// Finding request
    Request(Request),
    /// Storing request, response pair
    Store(Request, Response),
}

enum CatcherResponse {
    /// Request found (or not)
    Response(Request, Option<Response>),
    /// Completed storing response
    Stored(Response),
    /// Some error happend
    Error(crate::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatcherMode {
    Store,
    Use,
    Hybrid,
    Passive,
}

impl CatcherMode {
    pub fn does_find_response(&self) -> bool {
        matches!(self, Self::Use | Self::Hybrid)
    }

    pub fn does_abort_when_failed_to_find_response(&self) -> bool {
        matches!(self, Self::Use)
    }

    pub fn does_store_response(&self) -> bool {
        matches!(self, Self::Store | Self::Hybrid)
    }
}

// Arc<Queue>를 가지고 사용하면 됨
#[derive(Debug)]
pub struct Queue {
    request_sender: Option<Sender<CatcherRequest>>,
    response_receiver: Mutex<Receiver<CatcherResponse>>,
    handle: Option<JoinHandle<()>>,
    pub mode: CatcherMode,
}

impl Drop for Queue {
    fn drop(&mut self) {
        drop(mem::take(&mut self.request_sender));
        if let Some(handle) = mem::take(&mut self.handle) {
            handle.join().unwrap();
        }
    }
}

impl Queue {
    pub fn new(mut config: CatcherConfig) -> Self {
        let (request_sender, request_receiver) = mpsc::channel();
        let (response_sender, response_receiver) = mpsc::channel();
        let mut queue = Queue {
            request_sender: Some(request_sender),
            response_receiver: Mutex::new(response_receiver),
            handle: None,
            mode: CatcherMode::Hybrid,
        };
        let handle: thread::JoinHandle<()> = thread::spawn(move || {
            if config.initialize {
                config.connection.execute("CREATE TABLE IF NOT EXISTS transactions (
                    type TEXT NOT NULL,
                    method TEXT NOT NULL,
                    url TEXT NOT NULL,
                    headers TEXT NOT NULL,
                    content BLOB NOT NULL,
                    response BLOB NOT NULL,
                    compressed BOOLEAN NOT NULL DEFAULT FALSE
                )", ()).unwrap();
                config.connection.execute(
                    "CREATE INDEX IF NOT EXISTS transactions_idx ON transactions (type, method, url, content, json(headers))",
                    (),
                ).unwrap();
            }

            loop {
                let future = match request_receiver.recv() {
                    Ok(future) => future,
                    // sender hung up
                    Err(_) => break,
                };

                let response_future = match config.process_future(future) {
                    Ok(future) => future,
                    Err(error) => CatcherResponse::Error(create_error(error)),
                };
                response_sender.send(response_future).unwrap();
            }
        });
        queue.handle = Some(handle);
        queue
    }

    async fn get_future_response(&self, future: CatcherRequest) -> CatcherResponse {
        // 값을 보내기 전 미리 receiver의 값을 점유하고 값을 받으면 풂으로써 한 번에 하나의 요청만 처리되도록 함.
        let response_receiver = self.response_receiver.lock().unwrap();
        self.request_sender.as_ref().unwrap().send(future).unwrap();
        response_receiver.recv().unwrap()
    }

    pub(crate) async fn find_response(&self, request: Request) -> crate::Result<(Request, Option<Response>)> {
        if !self.mode.does_find_response() {
            return Ok((request, None))
        }

        let result = self.get_future_response(CatcherRequest::Request(request)).await;

        match result {
            CatcherResponse::Response(request, response) => {
                if response.is_none() && self.mode.does_abort_when_failed_to_find_response() {
                    return Err(create_error(anyhow::anyhow!("Response was not found.")));
                }
                Ok((request, response))
            },
            CatcherResponse::Error(err) => Err(err),
            _ => unreachable!(),
        }
    }

    /// response는 반드시 processed된 상태어야 합니다!
    pub(crate) async fn store_response(&self, request: Request, response: Response) -> crate::Result<Response> {
        if self.mode.does_store_response() {
            let result = self.get_future_response(CatcherRequest::Store(request, response)).await;
            match result {
                CatcherResponse::Stored(response) => Ok(response),
                CatcherResponse::Error(err) => Err(err),
                _ => unreachable!(),
            }
        } else {
            Ok(response)
        }
    }
}

pub struct CatcherConfig {
    connection: Connection,
    category: String,
    // 아마 feature로 빼는 것도 가능할 수도??
    check_headers: bool,
    initialize: bool,
}

fn header_map_to_json(header_map: &HeaderMap) -> anyhow::Result<Value> {
    let mut headers: serde_json::Map<String, Value> = serde_json::Map::new();
    for (name, value) in header_map {
        let name = name.as_str();
        let value = value.as_bytes();
        headers.insert(name.to_string(), Value::String(std::str::from_utf8(value)?.to_string()));
    }
    Ok(Value::Object(headers))
}

impl CatcherConfig {
    pub fn new(connection: Connection, category: String, check_headers: bool, initialize: bool) -> Self {
        Self { connection, check_headers, category, initialize }
    }

    pub fn build_queue(self) -> Queue {
        Queue::new(self)
    }

    fn process_future(&mut self, future: CatcherRequest) -> anyhow::Result<CatcherResponse> {
        let result = match future {
            CatcherRequest::Request(request) => {
                let response = self.find_response(&request)?;
                CatcherResponse::Response(request, response)
            },
            CatcherRequest::Store(request, response) => {
                self.store_response(&request, &response)?;
                CatcherResponse::Stored(response)
            }
        };
        Ok(result)
    }

    fn find_response(&mut self, request: &Request) -> anyhow::Result<Option<Response>> {
        let url = request.url().as_str();
        // body를 따로 설장하지 않은 경우에도 None이 나오는 듯? 아니면 GET은 body를 안 가져서 그런 건가?
        // NULL은 다른 NULL과 비교 시 NULL(false로 간주됨)이 나오기 때문에 반드시 null 대신 empty blob를 저장해야 함!
        let body = request.body().and_then(|body| body.as_bytes()).unwrap_or(b"");
        let method = request.method().as_str();
        let headers = serde_json::to_string(&header_map_to_json(request.headers())?)?;
        let category = self.category.as_str();

        if !self.check_headers {
            let mut select_matched_transaction = self.connection.prepare_cached(
            "SELECT response, compressed FROM transactions WHERE (
                    type = CAST(? AS TEXT)
                    AND method = CAST(? AS TEXT)
                    AND url = CAST(? AS TEXT)
                    AND content = CAST(? AS BLOB)
                )")?;
            let mut rows = select_matched_transaction.query_map(
                (category, method, url, body),
                |row| {
                    let closure: Result<Response, anyhow::Error> = (move || {
                        // row
                        // _, method, url, headers, content, response, compressed
                        let response: Vec<u8> = row.get_unwrap(0);
                        let compressed: u8 = row.get_unwrap(1);
                        if compressed != 2 {
                            bail!("Incompetible response type (Maybe came from the Python version)");
                            // bail!("Compressed response is not implemented.");
                        }
    
                        let response: SerializableResponse = postcard::from_bytes(&response)?;
                        let response: Response = response.try_into()?;
                        Ok(response)
                    })();
                    Ok(closure)
                }
            )?;
            // 해당하는 response가 없는 경우 None을 반환
            let result = rows.next().transpose()?.transpose()?;
            Ok(result)
        } else {
            let mut select_matched_transaction = self.connection.prepare_cached(
            "SELECT response, compressed FROM transactions WHERE (
                    type = CAST(? AS TEXT)
                    AND method = CAST(? AS TEXT)
                    AND url = CAST(? AS TEXT)
                    AND json(headers) = json(CAST(? AS TEXT))
                    AND content = CAST(? AS BLOB)
                )")?;
            let mut rows = select_matched_transaction.query_map(
                (category, method, url, &headers, body),
                |row| {
                    let closure: Result<Response, anyhow::Error> = (move || {
                        // row
                        // _, method, url, headers, content, response, compressed
                        let response: Vec<u8> = row.get_unwrap(0);
                        let compressed: u8 = row.get_unwrap(1);
                        if compressed != 2 {
                            bail!("Incompetible response type (Maybe came from the Python version)");
                        }
    
                        let response: SerializableResponse = postcard::from_bytes(&response)?;
                        let response: Response = response.try_into()?;
                        Ok(response)
                    })();
                    Ok(closure)
                }
            )?;
            // 해당하는 response가 없는 경우 None을 반환
            let result = rows.next().transpose()?.transpose()?;
            Ok(result)
        }
    }

    fn store_response(&mut self, request: &Request, response: &Response) -> anyhow::Result<()> {
        let url = request.url().as_str();
        let body = request.body().and_then(|body| body.as_bytes()).unwrap_or(b"");
        let method = request.method().as_str();
        let headers = serde_json::to_string(&header_map_to_json(request.headers())?)?;
        let category = self.category.as_str();
        let response = postcard::to_allocvec(&response.to_serializable()?)?;
        let compressed = 2;
        
        let mut insert_transaction = self.connection.prepare_cached("
            INSERT INTO transactions (type, method, url, headers, content, response, compressed) VALUES (
                CAST(? AS TEXT), CAST(? AS TEXT), CAST(? AS TEXT), CAST(? AS TEXT), CAST(? AS BLOB), CAST(? AS BLOB), CAST(? AS BOOLEAN)
            )
        ")?;
        insert_transaction.execute((category, method, url, headers, body, response, compressed))?;

        Ok(())
    }
}

pub(super) fn create_error(err: anyhow::Error) -> crate::Error {
    crate::error::catcher(err)
}
