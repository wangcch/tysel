# LLM service

This service routes `POST /generate` through one host-configured,
OpenAI-compatible Responses endpoint.

```sh
export OPENAI_API_KEY='replace-in-your-secret-manager'
export TYSEL_LLM_ENDPOINT='https://api.openai.com/v1/responses'
export TYSEL_LLM_MODEL='provider-model-name'
export TYSEL_LLM_ALIAS='default'

tysel check
tysel run
```

From another terminal:

```sh
curl -sS http://127.0.0.1:3000/generate \
  -H 'content-type: application/json' \
  -d '{"prompt":"Explain bounded capabilities in one sentence."}'
```

See the [LLM gateway guide](../../docs/guides/llm-gateway.md).
