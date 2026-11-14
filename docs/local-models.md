# Local model servers

In **Connections → Another compatible service**, choose the program's preset.
The address remains editable: use the port and API prefix shown by your server.
Press **Check connection**, save, then choose an installed model in Models.
If discovery is unavailable, add the server's exact model ID manually. Set Bearer authentication
and enter its key if you enabled authentication in the server.

## Ollama

Install [Ollama](https://docs.ollama.com/quickstart) once. Sovra checks its server
before model requests and starts `ollama serve` if the selected local server is
not running. **Check connection**, **Refresh** and **Download a model** also
start it. Merely opening Sovra with an unused Ollama connection does not launch a server.

Startup waits up to 30 seconds. A missing executable, a process that exits, a
wrong service on the port, or an unresponsive server produces a specific error.
Sovra never installs software or downloads a model unless you request a download.
Use **Download a model** for missing models; a running server alone does not mean
model weights are installed or that your computer has enough memory to load them.

Automatic startup supports HTTP on `localhost`, literal IPv4 loopback or `::1`,
including custom ports. Remote addresses, HTTPS and reverse-proxy paths remain
externally managed. Sovra binds a new process to the selected loopback address
and port. It finds Ollama on PATH or in common desktop installation locations.
Existing services remain running when Sovra closes; processes started by Sovra
are owned by its runtime and stop with the app.

## Other programs

Start the server and install/load a model in the chosen program first. These
presets all use the shared OpenAI-compatible adapter (`GET /models` and
`POST /chat/completions` relative to the API base). Replies stream by default,
except for GPT4All. Automatic process startup is currently provided for Ollama. Other programs need their own model paths, GPU
settings or desktop server switches.

| Preset | Suggested API base | Official setup/API reference |
| --- | --- | --- |
| LM Studio | `http://localhost:1234/v1` | [Server API](https://lmstudio.ai/docs/developer/openai-compat) |
| llama.cpp | `http://localhost:9931/v1` | [llama-server](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) |
| llama-cpp-python | `http://localhost:8000/v1` | [Server](https://llama-cpp-python.readthedocs.io/en/latest/server/) |
| LocalAI | `http://localhost:8080/v1` | [Quickstart](https://localai.io/docs/basics/getting_started/) |
| Jan | `http://127.0.0.1:1337/v1` | [Local API Server](https://docs.jan.ai/desktop/api-server) |
| GPT4All | `http://127.0.0.1:4891/v1` | [Enable Local API Server](https://docs.gpt4all.io/gpt4all_api_server/home.html) |
| vLLM | `http://localhost:8000/v1` | [Compatible server](https://docs.vllm.ai/en/latest/serving/openai_compatible_server/) |
| SGLang | `http://localhost:30000/v1` | [OpenAI API](https://docs.sglang.io/docs/basic_usage/openai_api_completions) |
| text-generation-webui | `http://localhost:5000/v1` | [API mode](https://github.com/oobabooga/textgen/wiki/12-%E2%80%90-OpenAI-API) |
| KoboldCpp | `http://localhost:5001/v1` | [Server wiki](https://github.com/LostRuins/koboldcpp/wiki) |
| MLX LM | `http://localhost:8080/v1` | [HTTP server](https://github.com/ml-explore/mlx-lm/blob/main/mlx_lm/SERVER.md) |
| MLC LLM | `http://localhost:8000/v1` | [REST server](https://llm.mlc.ai/docs/deploy/rest.html) |
| Docker Model Runner | `http://localhost:12434/engines/v1` | [Enable host TCP access](https://docs.docker.com/ai/model-runner/api-reference/) |
| Lemonade | `http://localhost:13305/v1` | [Server API](https://lemonade-server.ai/docs/api/) |
| Hugging Face TGI | `http://localhost:3000/v1` | [Messages API (1.4+)](https://huggingface.co/docs/text-generation-inference/reference/api_reference) |
| Xinference | `http://localhost:9997/v1` | [Using Xinference](https://inference.readthedocs.io/en/latest/getting_started/using_xinference.html) |

For TGI, the preset uses the documentation’s port-3000 example; match your own
container port mapping and add the served model ID manually if needed. Docker
Model Runner needs host TCP access enabled. Older Lemonade versions may use
port 8000 and `/api/v1`; use the API base shown by your installed version.

Use **Other OpenAI-compatible server** for additional implementations or proxies
that expose the same chat protocol. Compatibility means chat transport and
catalog discovery, not every server-specific feature. The GPT4All preset disables
streaming because its current server accepts complete replies only, and uses
the older `max_tokens` reply-limit field required by that server. Both settings
are saved with the connection. You can
change **Show the reply as it arrives** in technical options for any compatible
connection. Older llama.cpp versions and Docker examples may use port 8080. Tool calling, images,
structured output and embeddings depend on the model and server configuration;
Sovra preserves unknown capabilities instead of inventing support. Configure tool
parsers/chat templates in the server when needed. Program versions can change
ports, so the running server's address is authoritative.

## Slow first replies

Local presets start with a 600-second wait setting. Whole local chat turns allow
at least ten minutes, including for saved connections with the old 60-second
setting. The connection wait setting (5–600 seconds) also bounds catalog and
other single HTTP requests. This gives
cold model loading and helper calls time to finish. Stop still cancels a turn.
If a model repeatedly fails, check available RAM/VRAM, use a smaller model, and
check the server's own logs. Raising a timeout cannot fix a model that does not
fit in memory. Close other demanding workloads or choose a smaller installed
model when RAM, graphics memory or swap is exhausted.

An **input budget** error is different from a startup or connection timeout.
The question, saved context and enabled helpers must fit the model together.
Try a fresh conversation, reduce unneeded helpers in Settings, or choose a model
with a larger context. Changing the wait time does not increase context size.
