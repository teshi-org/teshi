## ADDED Requirements

### Requirement: Prefer HWND-scoped Windows Graphics Capture
On supported Windows x64 hosts, WinApp mode SHALL use Windows Graphics Capture with the exact attached HWND as the preferred preview and screenshot backend.

#### Scenario: WGC starts for an attached window
- **WHEN** a valid top-level HWND is attached and WGC produces its first frame within two seconds
- **THEN** the sidecar streams quality-70 JPEG frames from that window at no more than approximately 8 FPS

#### Scenario: Target window is occluded
- **WHEN** another window visually covers a target captured through WGC
- **THEN** preview frames continue to contain the target window's composited surface rather than the occluding window

### Requirement: Automatically fall back to ImageGrab
WinApp mode SHALL switch to screen-rectangle ImageGrab when WGC cannot be imported, initialized, or produce a first frame, or when its capture thread terminates while the target HWND remains valid.

#### Scenario: WGC dependency is unavailable
- **WHEN** the WGC package cannot be imported
- **THEN** attachment succeeds with ImageGrab and records the import failure as the fallback reason

#### Scenario: WGC first frame times out
- **WHEN** no WGC frame arrives within two seconds after attachment
- **THEN** the sidecar stops that WGC session and continues through ImageGrab

#### Scenario: Target window closes
- **WHEN** the active capture ends and the attached HWND is no longer valid
- **THEN** the sidecar emits a frame error instead of falling back to an unrelated screen rectangle

### Requirement: Bound capture lifecycle and frame storage
The sidecar SHALL stop the previous WGC session on reattachment or shutdown and SHALL retain at most the latest encoded WGC frame.

#### Scenario: A new target is attached
- **WHEN** a session already capturing one HWND attaches to another HWND
- **THEN** the old capture control is stopped before frames from the new target are exposed

#### Scenario: Producer outpaces broadcast
- **WHEN** multiple WGC callbacks arrive before the next broadcast interval
- **THEN** intermediate frames are replaced and only the newest JPEG remains available

### Requirement: Preserve the JPEG preview contract
Both capture backends SHALL supply the existing quality-70 JPEG/Base64 preview and screenshot interfaces.

#### Scenario: Existing client consumes a WGC frame
- **WHEN** a WGC frame is broadcast
- **THEN** it retains the existing `type`, `data`, `url`, `title`, and `seq` fields and remains decodable by clients unaware of backend metadata

