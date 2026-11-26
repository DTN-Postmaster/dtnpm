# List of Potential Evolutions to Evaluate

This document catalogs functional, architectural, and ergonomic evolution proposals for the DTN software suite (`dtn-hdy-utils` and the standalone `dtn-hdy-ham` server/BPA). Each proposal includes a preliminary evaluation detailing context, benefits, points of attention, and implementation recommendation.

---

## 1. Migration of the Markdown Engine to a Proven Library (`marked` + `DOMPurify`)

### Description and Use Case
Replace the custom regex parser ([`markdown.ts`](file:///home/loic/projets/dtn-hdy-utils/dtn-hdy-ham/web/src/utils/markdown.ts)) with the standard `marked` engine paired with the `DOMPurify` HTML sanitizer inside the Preact UI (`dtn-hdy-ham/web`).

### Expected Benefits
- **Strict CommonMark / GitHub Flavored Markdown (GFM) compliance**: Native, reliable support for tables, nested lists, blockquotes (`>`), complex links with nested parentheses, and adjacent punctuation.
- **Stronger, long-term XSS security**: `DOMPurify` is actively maintained and audited by web security specialists, protecting the UI against sophisticated bypasses (DOM clobbering, mXSS, malicious entities).

### Technical Challenges and Drawbacks
- **Web bundle size**: Adds approximately 25 to 35 KB (gzipped) to the embedded web UI bundle.
- **Custom URI scheme support**: Requires explicitly configuring `DOMPurify` (`ALLOWED_URI_REGEXP` or hooks) to allow the `dtn://` scheme, which is blocked by default.
- **Styling**: Requires a custom `marked` renderer to preserve existing Tailwind CSS classes.

### Evaluation Status
> **Recommended** if usage evolves toward exchanging structured notes or documentation (tables, blockquotes). The current regex implementation remains sufficient for simple chat and quick previews.

---

## 2. Dynamic Naming and Automatic Extension on File Download

### Description and Use Case
When downloading a received file without an extension or typed generically as `application/octet-stream`, use the MIME type auto-detected by the backend to generate a consistent suggested filename via the HTTP `Content-Disposition: attachment; filename="..."` header or the UI `download` attribute.

### Expected Benefits
- **Seamless user experience**: Avoids forcing the operator to open a `.bin` or extensionless file and manually guess the associated application (e.g., `.gpx`, `.sqlite`, `.svg`, `.pdf`).
- **OS consistency**: The user's host OS immediately associates the right application on double-click.

### Technical Challenges and Drawbacks
- Requires maintaining a robust reverse MIME -> default extension mapping table in `files_service.rs` and/or on the frontend.
- Handling collisions when the original filename already has a different extension from the detected one.

### Evaluation Status
> **High priority**: Low development cost, immediate usability value for radio operators receiving field files.

---

## 3. Integrated Map Preview for Geospatial Tracks (GPX, KML, GeoJSON)

### Description and Use Case
Integrate a lightweight modal or map pane (e.g., based on [Leaflet](https://leafletjs.com/) with locally cached or vector tiles) to preview GPX track files, KML markers, or GeoJSON layers received over DTN directly in the `dtn-hdy-ham` UI.

### Expected Benefits
- **Field operational relevance**: Enables emergency or rescue radio teams in dead zones to immediately view waypoints, progression tracks, or disaster areas on a map without third-party software.
- **Natural integration** with SenML telemetry and geolocated observation bundles.

### Technical Challenges and Drawbacks
- **Strict offline mode**: In dead zones / disaster response, no remote OpenStreetMap tiles are accessible over the Internet. Requires either offline basemap management (MBTiles / lightweight embedded raster) or pure vector track rendering (relative coordinates, elevation profile).
- Weight of the mapping library in the UI.

### Evaluation Status
> **To be evaluated**: High business value in amateur radio and emergency response, but requires defining the disconnected basemap strategy.

---

## 4. Secure PDF Document Preview in the UI

### Description and Use Case
Allow read-only viewing of PDF documents transmitted over DTN (newsletters, emergency procedures, weather) directly inside the station's web interface.

### Expected Benefits
- Quick consultation of procedures and bulletins without leaving the DTN management station.
- Avoids switching back and forth with an external reader on tablets or mobile devices.

### Technical Challenges and Drawbacks
- **Security**: Direct rendering via an `<iframe src="/api/files/download/...">` tag can expose vulnerabilities if the PDF contains embedded JavaScript. Requires strict sandboxing (restricted `sandbox="allow-scripts"`) or rendering via PDF.js in an isolated canvas.
- **Size**: Full PDF.js is large (~300 KB gzipped) for a low-power embedded station (Raspberry Pi).

### Evaluation Status
> **To be evaluated**: Test the feasibility of a simple browser-native sandboxed `iframe` before considering a dedicated library.

---

## 5. Archive Content Inspection and Browsing (ZIP, TAR, GZ)

### Description and Use Case
Display the list of contained files (directory tree, individual sizes) in the detail pane of an archive file without requiring prior local download and extraction.

### Expected Benefits
- Allows the operator to verify the contents of a file package before deciding to distribute or decompress it.
- Avoids manual steps to inspect a `dtnbasket` basket or a grouped bundle.

### Technical Challenges and Drawbacks
- Requires inspecting the archive header on the Rust backend (`zip` or `tar`) safely against "zip bombs" (limits on the number of inspected entries and tree depth).
- Adds lightweight server-side archive header parsing dependencies.

### Evaluation Status
> **Medium priority**: Very useful for operational transparency, straightforward to implement in Rust in a bounded manner.

---

## 6. Markdown Input Assistant Toolbar in Chat

### Description and Use Case
Add an ergonomic mini-toolbar above the Chat text input ([`ChatView.tsx`](file:///home/loic/projets/dtn-hdy-utils/dtn-hdy-ham/web/src/components/ChatView.tsx)) offering shortcuts: Bold (`**`), Italic (`*`), Code (`` ` ``), Markdown Link (`[name](url)`), and quick insertion of an EID identifier (`dtn://...`).

### Expected Benefits
- Reduces Markdown syntax typing errors on mobile or field touchscreens.
- Standardizes message formatting without requiring the user to memorize syntax.

### Technical Challenges and Drawbacks
- Ergonomics on small screens (tablet/smartphone in PWA mode): must remain unobtrusive and not obscure chat history when the virtual keyboard opens.

### Evaluation Status
> **High priority**: Easy to integrate into the existing Preact component, direct usability gain.

---

## 7. Transparent Outbound Compression for Slow Radio Links

### Description and Use Case
Offer a transparent compression option (e.g., Zstandard or Deflate) for large bundle payloads (text files, JSON, SenML, logs) prior to transmission over very low-bitrate radio CLAs (Mercury TNC, KISS HF/VHF, LTP).

### Expected Benefits
- Significant reduction in spectrum occupancy time (radio channel) for text and telemetry streams.
- Reduced power consumption for radio transmitters during extended transmissions.

### Technical Challenges and Drawbacks
- Must be negotiated or signaled at the bundle metadata or MIME type level so the receiving node knows to decompress automatically.
- Useless (or counterproductive) on already-compressed files (JPEG, PNG, MP4, ZIP archives).

### Evaluation Status
> **To be planned**: Requires specification at the DTN application protocol level or CLA profile to guarantee interoperability.

---

## 8. Standardized Emergency Forms Support (Winlink / ICS-213 Style)

### Description and Use Case
Enable filling, sending, receiving, and viewing standardized operational emergency forms (widely used in the Winlink Standard Forms ecosystem, ARES, RACES, and ADRASEC civil defense networks):
- **ICS (Incident Command System) Forms**: ICS-213 (General Message), ICS-214 (Activity Log), ICS-205 (Radio Plan).
- **Situation and Check-in Reports**: Station Check-in / Check-out, simplified weather report, Field Situation Report.

The system relies on a bandwidth-frugal hybrid model:
1. Form templates (HTML/JSON) pre-embedded in the station's web UI.
2. Ultra-compact radio exchange: only the entered data (a few hundred bytes of JSON or CBOR) transits in the DTN bundle payload to a dedicated endpoint (e.g., `dtn://station/forms` or Chat/Files service).
3. Faithful visual reconstruction upon reception matching the regulatory form layout, with clean print or PDF export options.

### Expected Benefits
- **Interoperability and operational adoption**: Rescue services and authorities use the ICS framework. Direct integration in `dtn-hdy-ham` positions the solution as a resilient store-and-forward alternative to Winlink.
- **Optimal spectral frugality**: Unlike sending a PDF or screenshot, only useful fields are transmitted over HF/VHF links (a 50x to 100x reduction in transmission time).
- **Validation and guidance**: Required field checks, GPS coordinates from the station, certified UTC timestamps.
- **BPSec authentication**: Ability to cryptographically sign the form with the station's key (via the BPSec/BIB module already built into `dtn-hdy-ham`) to guarantee the integrity of mission orders and reports.

### Technical Challenges and Drawbacks
- **Data format (Winlink interop vs Native CBOR/JSON)**: Evaluate whether to parse/generate Winlink Express's standard XML dialect to build direct gateways, or adopt a lighter standardized JSON/CBOR format.
- **Template catalog management**: Mechanism to update templates without recompiling the entire application (e.g., customizable templates folder).
- **Degraded display**: If a station receives a form whose local template it does not possess, a generic "key / value" table fallback is required.

### Evaluation Status
> **High priority / Strategic**: Major evolution with high added value for emergency communications and the amateur radio community.

---

## 9. Formal Verification and Panic-Free Guarantees (`no-panic` & `cargo-kani`)

### Description and Use Case
In a Delay-Tolerant Network (DTN), and especially in emergency or isolated radio infrastructure contexts, a process crash (`panic!`) on an intermediate node or standalone station interrupts storage and forwarding for all in-transit bundles. Beyond static linting rules (`clippy::unwrap_used` and `clippy::panic`), the goal is to integrate mathematical and link-time verification tools proving the absolute absence of panics on critical execution paths:

1. **Option A: `#[no_panic]` Link-Time Attribute (David Tolnay)**:
   * Allows annotating critical low-level functions (packet parsers, network decoders, cryptographic functions).
   * How it works: Relies on LLVM compiler optimizations in `--release` mode (dead code elimination). If the compiler cannot prove a panic path is unreachable, linking fails immediately with a missing symbol (`__no_panic`).
   * *Ideal targets*: LTP segment parsing (`hardy-ltp`), SenML CBOR decoder (`senml/parser.rs`), CoAP/CBOR basket metadata deserialization (`dtnbasket`).

2. **Option B: Bounded Model Checking with Kani (`cargo-kani`, AWS)**:
   * Bit-precise formal verifier backed by SMT/SAT solvers (CBMC).
   * How it works: Symbolically explores the infinite set of input byte combinations on a formal test harness to mathematically prove:
     * Zero out-of-bounds dereferencing (`slice out-of-bounds` on frame indices).
     * Zero arithmetic overflow (`integer overflow`).
     * Zero implicit panics (`Option::unwrap()` or `Result::unwrap()`).
   * *Ideal targets*: LTP reassembly finite state machine (`SessionFsm`), segment claim calculation, BPSec/BIB verification engine (`keystore.rs`).

3. **Option C: Static Call-Graph Analysis (`cargo-call-stack` / Symbol Inspection)**:
   * With `panic = "abort"` in the release profile, verifying that the call tree of compiled binaries (`dtn-hdy-ham`, CLAs) does not reference internal panic symbols `core::panicking::panic` or `rust_begin_unwind`.

### Expected Benefits
- **Absolute resilience on noisy radio links**: Corrupted or fuzzed frames received over RF can under no circumstances crash the daemon.
- **Zero silent regressions**: Any added code that could panic is caught either at compile time (link-time) or by formal solvers in CI.
- **Zero runtime overhead**: Unlike dynamic defensive guards (`catch_unwind`), these approaches rely on static elimination.

### Technical Challenges and Drawbacks
- **`#[no_panic]`**: Requires optimized builds (`--release`), because in debug mode without LLVM optimization, the compiler does not simplify index check branches.
- **Kani**: SMT solvers require longer CI analysis times (tens of seconds per harness) and impose explicit *unwind bounds* on dynamic loops.

### Evaluation Status
> **Recommended**: Progressive integration starting with `#[no_panic]` on pure binary parsing functions in `hardy-ltp` and `senml`, then writing Kani harnesses for the LTP session FSM.

---

## 10. Configuration Override via Environment Variables (`DTN_HDY_HAM__*`)

### Description and Use Case
In accordance with Hardy's architecture guidelines (`hardy/docs/design.md` §7), allow dynamic overriding of any configuration key via environment variables prefixed with `DTN_HDY_HAM__` using a double underscore `__` separator for nested hierarchy (e.g., `DTN_HDY_HAM__CALLSIGN=F4XYZ`, `DTN_HDY_HAM__STORAGE__IN_MEMORY=true`, `DTN_HDY_HAM__WEB__ADDRESS="0.0.0.0:8088"`).

### Expected Benefits
- **Simplified containerized deployments (Docker, Podman, Kubernetes)**: Allows configuring a complete amateur radio DTN station via a `docker-compose.yml` file or system environment variables without injecting or mounting a configuration file volume.
- **Systemd integration**: Enables using `EnvironmentFile=/etc/default/dtn-hdy-ham` in the systemd unit for infrastructure deployments.
- **Consistency with the Hardy ecosystem**: Aligns `dtn-hdy-ham` with the standard behavior of `hardy-bpa-server` and `hardy-tcpclv4-server` binaries.

### Technical Challenges and Drawbacks
- Requires adding the `config::Environment::with_prefix("DTN_HDY_HAM").separator("__")` source in the `HamConfig::load` configuration builder.
- Must preserve precedence: defaults < configuration file < environment variables < command-line arguments (CLI).

### Evaluation Status
> **High priority / Easy**: Very low implementation cost in `HamConfig::load`, immediate operational gain for containers and automated deployments.

---

## 11. File-Based CLA Integration (`hardy-file-cla` / Sneakernet)

### Description and Use Case
Integrate Hardy's official `hardy-file-cla` crate into `dtn-hdy-ham` to enable exchanging and relaying BPv7 bundles through filesystem directories (inbound `inbox` and outbound `outbox` folders), ideal for physical transport ("Sneakernet") on USB drives, SD cards, or synchronized shared folders (Syncthing, Nextcloud, rsync).

### Expected Benefits
- **Absolute resilience during total blackout**: Even without any available HF/VHF radio link or IP connectivity, two stations can exchange messages, emergency files, and telemetry by simply transferring a USB drive physically.
- **Hybrid Air-Gap Gateway**: Enables an isolated station not connected to the Internet to transfer bundles to a gateway via removable media.
- **Standardization**: Directly reuses the official `hardy-file-cla` component maintained in the Hardy tree.

### Technical Challenges and Drawbacks
- Directory watching (polling vs inotify/kqueue notifications).
- File permissions handling on removable media (FAT32/exFAT filesystems without POSIX permissions).
- Web UI integration (displaying the file CLA status and the count of dropped/read bundles).

### Evaluation Status
> **Implemented (`cla-file` feature, [ADR 23](../adr/0023-file-based-cla-security-and-sneakernet-spooling.md))**: `hardy-file-cla` is integrated in `dtnpm` (`type: "file-cla"` / `"file"`, multi-instance support, tilde expansion, automatic `FILE` + custom APRS aliases, fail-closed `0700` spool dirs with `allow-permissive-spool` opt-in, default `64 MiB` `max-bundle-size` DoS cap, path-safety registry, and dry-run `dtnpm check-config`). Future enhancements track upstream `hardy-file-cla` hardening (atomic `.tmp`→rename writes, `O_NOFOLLOW`, USB hotplug mount recovery) and WebUI spool counters.
