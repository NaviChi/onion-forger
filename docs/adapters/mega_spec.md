# Mega.nz Adapter Specification (Phase 52A)

## Architecture Overview
The Mega handler provides native Node tree parsing and AES-128-CTR decryption for architectures like **NightSpire**, which deploy standard clearnet Mega.nz distributed shared folders.

## Core Mechanisms
1. **URL Detection (`is_mega_link`, `is_mega_protected_link`)**
   - Sniffs `mega.nz` and legacy `mega.co.nz` domains and halts generic HTTP routing.
2. **Key Extraction & Validation**
   - Plucks the `#KEY` fragment from the URL format `https://mega.nz/folder/HANDLE#KEY`. Fail-fast trigger if the hash/key is missing.
3. **Decryption API (`mega` crate)**
   - Interfaces directly with Mega.nz to execute AES-128-CTR folder tree decryptions entirely in memory, mapping internal `NodeKind::Folder` and `NodeKind::File` traits to standard `FileEntry` matrices.

## Prevention Rules
- **PR-MEGA-001**: Execution keys must exist entirely in-memory and must never persist explicitly to `.crawli_vtdb`.
- **PR-MEGA-002**: Trigger fail-fast UI responses if the URL lacks the secondary decrypting half.
