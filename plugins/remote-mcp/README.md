# Remote MCP

Gives a sandboxed agent a remote MCP server's tools, over HTTP, through Silt: the agent never holds the token, and each request Silt sends is the one shown.

```bash
silt plugin set silt.remote-mcp url https://mcp.acme.dev/mcp
silt plugin secret silt.remote-mcp token < token.txt
silt plugin enable silt.remote-mcp --workspace work
```

`remote_mcp_tools` lists the server's tools, and `remote_mcp_call` calls one with its arguments. Calls ask until you allow a tool: `silt plugin rule silt.remote-mcp call allow --where tool=search`.
