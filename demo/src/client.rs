use anyhow::{Context, Result, bail};
use reqwest::{Client, Method};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    http: Client,
}

impl ApiClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: Client::new(),
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str, api_key: Option<&str>) -> Result<T> {
        self.request(Method::GET, path, api_key, None::<&()>, None).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        api_key: Option<&str>,
        body: &B,
        idempotency_key: Option<&str>,
    ) -> Result<T> {
        self.request(Method::POST, path, api_key, Some(body), idempotency_key).await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str, api_key: Option<&str>) -> Result<T> {
        self.request(Method::DELETE, path, api_key, None::<&()>, None).await
    }

    async fn request<T: DeserializeOwned, B: Serialize>(
        &self,
        method: Method,
        path: &str,
        api_key: Option<&str>,
        body: Option<&B>,
        idempotency_key: Option<&str>,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let mut request = self.http.request(method, &url);

        if let Some(api_key) = api_key {
            request = request.bearer_auth(api_key);
        }
        if let Some(idempotency_key) = idempotency_key {
            request = request.header("Idempotency-Key", idempotency_key);
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request.send().await.with_context(|| format!("request failed for {url}"))?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            bail!("{} {}", status, text);
        }

        serde_json::from_str(&text).with_context(|| format!("invalid JSON response from {url}"))
    }
}
