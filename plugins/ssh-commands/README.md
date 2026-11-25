# SSH commands

Lets a sandboxed agent run a command on a host you name, such as a car on a test fleet, through this machine's own `ssh`: your keys, agent and known hosts, in batch mode, with no terminal, forwarding or local command.

```bash
silt plugin set silt.ssh-commands targets '["*.fleet.vpn"]'
silt plugin environment silt.ssh-commands fleet "*.fleet.vpn"
silt plugin enable silt.ssh-commands --workspace work
```

Each run is asked, showing the host, the command and its input. Silt can't check what a command does once it runs, so the card says so, and an org can refuse such calls altogether.
