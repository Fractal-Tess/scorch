# Scorch Python client

A synchronous, dependency-free, typed Python client for the Scorch HTTP API. It supports Python 3.11 and newer.

## Install from this repository

```sh
python -m pip install ./clients/python
```

The package is not yet published to PyPI.

## Use

```python
from scorch_client import ScorchClient

client = ScorchClient()  # http://127.0.0.1:33000

response = client.search(
    "времето в София",
    country="bg",
    language="bg",
    engines=["bing", "duckduckgo"],
    limit=5,
)

for result in response["results"]:
    print(result["title"], result["url"])

page = client.scrape(
    "https://example.com",
    options={"formats": ["markdown", "links"], "render": "auto"},
)
print(page.get("markdown", ""))
```

Mapping and crawling use the same client:

```python
site = client.map("https://example.com", limit=100)

job = client.start_crawl("https://example.com", limit=20, max_depth=2)
page = client.crawl_status(job["id"], cursor=0, page_size=10)
client.cancel_crawl(job["id"])
```

Set `SCORCH_API_URL` or pass `base_url` for another service. Static headers support authenticated gateways without putting credentials in the URL:

```python
client = ScorchClient(
    "https://scorch.example.com",
    headers={"Authorization": "Bearer ..."},
    timeout=135,
)
```

## Errors and limits

- `ScorchAPIError` exposes `status`, `code`, `message`, and `request_id`.
- `ScorchConnectionError` reports connection and timeout failures.
- `ScorchResponseError` reports invalid JSON and oversized responses.
- Responses are streamed into a bounded 64 MiB buffer by default.
- One absolute client deadline covers connection, headers, and body reads.
- Internal transport workers are capped at eight so stalled DNS lookups cannot create unbounded threads.
- API redirects are rejected so gateway credentials cannot be forwarded to another origin.

## Test

```sh
cd clients/python
python -m pip install --editable .
python -m unittest discover -s tests -v
```
