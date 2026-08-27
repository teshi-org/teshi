## ADDED Requirements

### Requirement: Capture backend status
WinApp target and frame metadata SHALL expose the active capture backend, and native and WASM GPUI previews SHALL distinguish WGC streaming from ImageGrab fallback without changing frame rendering.

#### Scenario: WGC frame is displayed
- **WHEN** a preview client receives a frame with `capture_backend` equal to `wgc`
- **THEN** it displays a live Windows Graphics Capture status and does not instruct the user to keep the target unobscured

#### Scenario: Fallback frame is displayed
- **WHEN** a preview client receives a frame with `capture_backend` equal to `imagegrab`
- **THEN** it displays the fallback reason and warns that the target must remain visible and unobscured

#### Scenario: Legacy frame omits backend metadata
- **WHEN** a client receives a valid frame without capture backend fields
- **THEN** it renders the JPEG and uses the generic legacy streaming status
