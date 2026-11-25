# Local MCP

Wraps an MCP server that runs on this machine, such as one from npm, so a sandboxed agent gets its tools while the server runs in a sandbox of its own: its own folder, no network but the hosts you set, through Silt's gateway.

```bash
silt plugin set silt.local-mcp command "npx -y @acme/mcp-server"
silt plugin set silt.local-mcp hosts '["api.acme.dev"]'
silt plugin enable silt.local-mcp --workspace work
silt plugin rule silt.local-mcp call allow --where tool=search
```

The server's tools are offered as `local_mcp_<tool>`. Each call asks until you allow its tool; Silt shows the arguments, but can't check what the server does with them.
