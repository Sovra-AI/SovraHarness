# Network access and local operation

Sovra stores workspace data on this computer, but model calls and connected tools
can send content elsewhere. Choosing a local chat model does not disable other
connections. Data locations and credentials are described in
[Configuration](configuration.md) and [Security](../SECURITY.md).

## Network destinations

This inventory covers the application integrations in the current code. Custom
provider URLs, MCP servers, terminal commands, and generated websites can contact
additional destinations; it is not a network allowlist.

| Connection | When it is used | Data and controls |
| --- | --- | --- |
| OpenRouter (`openrouter.ai`) | Catalog refresh for the default connection; inference after selecting its model | Catalog metadata and, for inference, model inputs. Remove the connection to stop its catalog refresh; choose another model to stop using its inference. |
| Ollama (`localhost:11434` by default) | Catalog discovery, inference, optional embeddings, explicit model pulls | Content goes to the configured server. A model pull asks that server to download a model; its downloads may use the Internet. |
| OpenAI-compatible and Anthropic endpoints | When configured and used | Model inputs go to the configured URL. Check the destination before saving a connection. |
| Gemini | Agent fallback when `GEMINI_API_KEY` or `GOOGLE_API_KEY` is configured | Model inputs. Remove unused environment credentials. |
| Laya (`127.0.0.1:8011`) or OpenJev (OpenRouter decisions endpoint) | Routing evaluation or simulation | Routing prompt. The default Laya address is local. Select **Embedded Rust Heuristic** to avoid the decision request; routing settings reset at restart. |
| MWMBL, English Wikipedia, optional SearXNG | Helper web search; scheduled research jobs | Search queries. Ordinary helper web search starts enabled; turn it off in helper settings. Review or remove scheduled research separately. |
| Public web pages | Reading search results | Requests to the selected URLs. The web reader checks addresses and refuses private/loopback destinations. |
| Google OAuth and APIs | Sign-in and connected Google tools | Consent, drafts, events, tasks, and requested files. Disconnect or disable individual services. |
| GitHub OAuth and REST | Device sign-in, token validation, and GitHub tools | Account/repository operations. Disconnect in Sovra; revoke the token on GitHub to remove server-side access. |
| WhatsApp Web and Telegram Bot API | While linked, including incoming messages and approved sends | Messaging content and account/session information. Unlink to stop the connection. |
| Group peers, discovery, and encrypted relays | Group transport startup and synchronization | Peer metadata and encrypted group traffic. `SOVRA_GROUP_REACH=local` removes Internet discovery and relays, but still permits direct peer traffic. |
| Group training peers | After **Start with my group** and another member's **Help this computer** | Chosen training text batches, model parameters and updates over the group connection. **Stop sharing**, **Stop helping** and the Groups training permission revoke access. Sharing and helping start off each launch. See [Training](training.md#what-is-shared). |
| Phone-control peers, discovery, and encrypted relays | While phone access is enabled | Remote-control traffic. Access starts off each launch. `SOVRA_REMOTE_REACH=local` limits reach; it does not enable access. |
| Remote MCP servers | Reconnecting configured services and calling enabled tools | Tool arguments and configured credentials. Disable or remove the service. Local MCP programs have their own network access. |
| jsDelivr | Opening a Mermaid artifact in the browser | The browser downloads the Mermaid script. Review generated web content for its own external resources. |

Implementation: [composition root](../crates/sovra-app/src/main.rs),
[provider adapters](../crates/sovra-providers/src),
[search sources](../crates/sovra-adk/src/web_search/sources.rs),
[decision client](../crates/sovra-adk/src/decision_client.rs),
[group transport](../crates/sovra-mesh/src), and
[browser previews](../crates/sovra-core/src/artifacts.rs).

## Configure local operation

1. Install and start a local model server with the models you need already available.
2. Choose its local connection in Models and remove unused remote connections,
   including the default OpenRouter connection if you do not want catalog requests.
3. Disable helper web search and remove scheduled research tasks.
4. Choose the embedded router if you do not want decision-server requests.
5. Leave unused cloud credentials and `SOVRA_SEARXNG_URL` unset.
6. Disconnect messaging, Google, GitHub, and remote MCP services. Review local
   MCP programs and terminal commands separately; they are not network sandboxes.
7. If enabling meaning search, select a local embedding endpoint as well.
8. Set `SOVRA_GROUP_REACH=local`; do not join a group if you want no peer traffic.
   Keep phone access off. Avoid browser previews that load external resources.

These settings control known integration paths. They do not provide a global
"offline mode" or prevent arbitrary commands or web content from networking.
Verify traffic in your deployment environment before relying on isolation.
The app has installer hooks but no in-app update-check flow.
