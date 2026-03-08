# DragonForce Adapter Guidance

## Architecture
DragonForce utilizes a nested `.onion` architecture. The external splash page acts as a proxy that loads a secondary `fsguest` iframe.
- **Root Domain:** e.g. `dragonforxx...onion`
- **IFrame API:** e.g. `fsguestuctex...onion`

## JWT Constraints
Every file and directory in the `fsguest` backend is secured by a dynamic JSON Web Token (JWT) passed in the URL schema (`?path=...&token=eyJhbGci...`).
- The Crawler must extract the `exp` (expiration) Unix timestamp from the base64-encoded payload.
- Before fetching a node from the queue, if the token is expired, the adapter should drop the request to prevent upstream 403 blocks and bandwidth waste.

