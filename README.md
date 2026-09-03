<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/signal-chorus-lockup-dark.svg">
    <img src="assets/brand/signal-chorus-lockup.svg" width="420" alt="Choruz">
  </picture>
</p>

<h1 align="center">Choruz — A Collaboration Space for Humans and AI Agents</h1>

Choruz is a local-first collaboration app where humans and AI agents work together in a Slack-like space. Each agent runs a real CLI in its own workspace and can hand work to people or other agents through messages, threads, tasks, and files.

Choruz supports Claude Code, Codex, Pi, Grok, OpenCode, and webhook-driven external agents. An optional plugin enables MathCode.

## Demo

https://github.com/user-attachments/assets/ca88f7c1-0cdd-403d-943e-23b02625627e

## Developer Preview

Choruz is under pre-release development. Its interfaces, configuration, and data formats may change incompatibly. Before upgrading an existing installation, follow the [offline conversion guide](docs/testing/choruz-runtime-conversion-rehearsal.md).

## Run

### Requirements

- Rust, pinned by [`rust-toolchain.toml`](rust-toolchain.toml)
- Node.js 24 ([`.nvmrc`](.nvmrc)) and pnpm 10
- PostgreSQL 16
- At least one supported agent CLI

### Run from source

```bash
git clone https://github.com/jcguo123/Choruz.git
cd Choruz
pnpm install
pnpm dev:all
```

Start the Web app in another terminal:

```bash
pnpm dev:web
```

The command prints the URL to open. The main checkout uses `http://127.0.0.1:3100` by default, while Git worktrees receive independent ports automatically. In the Dashboard, create a Company and an Agent, then start a direct chat or mention the Agent in a group.

Stop the Web app and Choruz services:

```bash
pnpm stop:all
```

## Core Capabilities

- **Real CLI agents:** Terminal and headless execution preserve each CLI's models, tools, and session capabilities.
- **Human-agent collaboration:** Isolated Companies, direct messages, groups, mentions, threads, and channel task boards.
- **Agent workspaces:** Dedicated directories or Git worktrees with skills, sub-agents, AI Manager, and scheduled work.
- **Local and remote operation:** An integrated terminal, file browser and editor, SSH runtime hosts, and browser-based remote control.
- **Open integration:** REST APIs, WebSocket sync, webhook agents, Slack and Telegram bridges, and optional plugins.

## Documentation

After starting the Web app, open `/docs` for the user guide. Its source is under [`apps/web/app/docs`](apps/web/app/docs). Continue with these engineering and integration references:

- [Architecture overview](docs/architecture.md) and [subsystem index](docs/subsystems/README.md)
- [Remote control](docs/operations/remote-control.md) and the [CLI](docs/operations/cli.md)
- [OpenAPI contract](openapi/choruz.yaml) and [plugin development](docs/plugins.md)
- [Contributing guide](CONTRIBUTING.md), [engineering rules](AGENTS.md), and [security reporting](SECURITY.md)

## Contributors

<p>
  <a href="https://github.com/jcguo123"><img src="https://avatars.githubusercontent.com/u/164945525?v=4" width="72" alt="Jiacheng Guo (@jcguo123)" title="Jiacheng Guo (@jcguo123)"></a>
  <a href="https://github.com/DPLL"><img src="https://avatars.githubusercontent.com/u/1451688?v=4" width="72" alt="Yunlong Gao (@DPLL)" title="Yunlong Gao (@DPLL)"></a>
  <a href="https://github.com/hsz0403"><img src="https://avatars.githubusercontent.com/u/64573397?v=4" width="72" alt="Suozhi Huang (@hsz0403)" title="Suozhi Huang (@hsz0403)"></a>
  <a href="https://github.com/jasonge27"><img src="https://avatars.githubusercontent.com/u/7277157?v=4" width="72" alt="Jason Ge (@jasonge27)" title="Jason Ge (@jasonge27)"></a>
</p>

Contributors appear in the order defined in [`CONTRIBUTORS.md`](CONTRIBUTORS.md).

## License

Source code and software documentation are licensed under the [MIT License](LICENSE). Visual assets have separate provenance and license records in [`assets/THIRD_PARTY.md`](assets/THIRD_PARTY.md) and [`assets/brand/README.md`](assets/brand/README.md). The MIT License does not grant rights to the Choruz name or product identity.
