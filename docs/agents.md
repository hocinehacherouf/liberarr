# AI agents (MCP)

`liberarr mcp` lets an AI agent read your recordings, so you can ask it "what did we decide in last week's meeting?". It is a [Model Context Protocol](https://modelcontextprotocol.io) server: your agent starts it, sends requests on its standard input and reads the answers on its standard output. It can only read; it changes nothing and never runs a pipeline step.

## Before you connect it

**What your agent reads goes to its AI provider.** A summary or a transcript that the agent asks for is sent to the model your agent uses (Anthropic for Claude Code, OpenAI for Codex, and so on), under that account's privacy settings. Connect it only to an agent whose provider you would trust with your meetings.

- **What it can see:** every recording's title, date, duration and status, the summaries and action items (only when a summary provider other than `none` is set), and the transcripts. Not the audio, the device serials, the keys or the settings.
- **Approve the calls.** Most agents ask before they call a tool; some run read-only tools without asking (for example Codex in its `writes` approval mode). Keep the approval prompts on if you want to see each read.
- **A transcript is untrusted text.** Anyone in a meeting can say "ignore your instructions and…", and the agent reads it. Liberarr tells the agent to treat recording text as data, which is only a soft guard: the real limit is that Liberarr offers no tool that changes anything. Be careful with what else the agent can do in the same session (web access, files, commands).

## Connect your agent

The agent starts `liberarr mcp` with your `LIBERARR_HOME` (leave it out if you use the default `~/.liberarr`). Use the full path of the binary if `liberarr` is not in the agent's `PATH`.

**Claude Code**

```sh
claude mcp add --env LIBERARR_HOME=/path/to/liberarr-home --transport stdio liberarr -- liberarr mcp
```

**Codex CLI**

```sh
codex mcp add liberarr --env LIBERARR_HOME=/path/to/liberarr-home -- liberarr mcp
```

or in `~/.codex/config.toml`:

```toml
[mcp_servers.liberarr]
command = "liberarr"
args = ["mcp"]
env = { LIBERARR_HOME = "/path/to/liberarr-home" }
```

**Cursor**: in `~/.cursor/mcp.json` (all projects) or `.cursor/mcp.json` (one project):

```json
{
  "mcpServers": {
    "liberarr": {
      "type": "stdio",
      "command": "liberarr",
      "args": ["mcp"],
      "env": { "LIBERARR_HOME": "/path/to/liberarr-home" }
    }
  }
}
```

**Other agents**: add a local (stdio, "command") server that runs `liberarr mcp` with `LIBERARR_HOME` in its environment.

It works while `serve`, `tui` or a pull runs: it only reads the database.

## Tools

| Tool | Gives |
|---|---|
| `list_recordings` | Recordings, newest first: id, title, local date and time, duration, status. Filters: `query` (title text, accents ignored), `since`, `until` (`YYYY-MM-DD`). Pages of 20 (`limit` up to 50), with `next_cursor`. |
| `search_transcripts` | Where something was said: the matching transcript segments (your corrections included), case and accents ignored, newest first, with the time and an excerpt. Same filters and pages as `list_recordings`; `query` is required (2 to 200 characters). |
| `get_summary` | The summary and the action items of one recording (`id`). |
| `get_transcript` | The transcript of one recording (`id`), in pages of about 20 KB of timed segments, with `next_cursor`. |
| `list_open_actions` | Action items not ticked yet, across recordings, with owner and due date. Same filters and pages as `list_recordings`. |

To tick an action or run a step again, use the web UI or the terminal app.

## Audit log

Each tool call adds one line to `<LIBERARR_HOME>/mcp-audit.log` (readable only by you): the time, the tool, the recording id, and whether it succeeded. It never holds the search text or any content. The file only grows; delete it whenever you like.

## Problems

- **The agent says the server failed to start.** Run `liberarr status` with the same `LIBERARR_HOME`: `liberarr mcp` needs the same configuration as the other commands, and prints the same errors on its standard error.
- **Typed `liberarr mcp` in a terminal and nothing happens.** That is normal: it waits for an agent's requests. Press Ctrl-D.

Next: [Privacy](privacy.md).
