## ADDED Requirements

### Requirement: Shared GPUI WinApp preview

The shared GPUI UI SHALL display the latest valid JPEG frame received from the WinApp sidecar and MUST preserve the captured frame's aspect ratio within the available preview area.

#### Scenario: Native shell receives a frame

- **WHEN** the native GPUI shell receives a valid WinApp `frame` message
- **THEN** the main surface displays that JPEG frame without stretching its aspect ratio

#### Scenario: WASM shell receives a frame

- **WHEN** the GPUI WASM shell receives a valid WinApp `frame` message through the browser WebSocket API
- **THEN** the same shared preview view displays that JPEG frame

### Requirement: Preview connection states

The preview SHALL distinguish connecting, waiting for the target or first frame, streaming, and failed states. A sidecar or frame error MUST be shown to the user without removing the last successfully displayed frame.

#### Scenario: Attached target has not produced a frame

- **WHEN** the WebSocket is connected but no valid frame has arrived
- **THEN** the preview shows a waiting state that identifies the intended WinApp target

#### Scenario: Stream reports an error after a frame

- **WHEN** a `frame_error` message arrives after at least one valid frame
- **THEN** the preview retains the last frame and displays the stream error

### Requirement: Bounded latest-frame handling

The prototype SHALL replace superseded frames instead of accumulating an unbounded frame queue.

#### Scenario: Producer outpaces rendering

- **WHEN** multiple frames arrive before GPUI renders the next update
- **THEN** intermediate frames MAY be discarded and the newest available frame is retained

### Requirement: Configurable prototype attachment

The prototype adapters SHALL request attachment to the configured target process after starting or connecting to the WinApp sidecar, and native desktop SHALL allow the process name to be overridden for development.

#### Scenario: Target application is running

- **WHEN** the prototype connects while a visible configured target process window exists
- **THEN** it requests attachment by process name and begins waiting for preview frames

#### Scenario: Target application is not running

- **WHEN** no matching visible process window exists
- **THEN** the preview displays the attachment failure rather than silently remaining blank
