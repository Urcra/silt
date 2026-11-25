# GraphQL API

Gives a sandboxed agent your GraphQL API through Silt: `graphql_query` for reads, `graphql_mutate` for changes, each to an environment you set up. The agent never holds the token, and Silt reads each document itself: a mutation sent as a query is refused.

## Setting it up
```bash
silt plugin environment silt.graphql prod prod
silt plugin set silt.graphql endpoint https://api.acme.dev/graphql --environment prod
silt plugin secret silt.graphql token --environment prod < token.txt
silt plugin enable silt.graphql --workspace work
```

## What decides
Queries are allowed and mutations asked, until you set your own rules: `silt plugin rule silt.graphql mutate never --where environment=prod`. Each mutation is approved as exactly the document and variables shown, and sent once.

## Data kept from agents
With `silt plugin set silt.graphql data '["location"]' --environment prod`, a request selecting fields such as `lastPosition` or `lat` is refused before it's sent, and an answer carrying coordinates is kept from the agent, whoever approved the call.
