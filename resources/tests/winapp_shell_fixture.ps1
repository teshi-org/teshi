param([Parameter(Mandatory=$true)][string]$StateDir)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Teshi Shell Acceptance'
$form.Width = 420
$form.Height = 200
$label = New-Object System.Windows.Forms.Label
$label.Text = 'Teshi visibility marker'
$label.AutoSize = $true
$form.Controls.Add($label)
$trayIcon = New-Object System.Windows.Forms.NotifyIcon
$trayIcon.Icon = [System.Drawing.SystemIcons]::Application
$trayIcon.Text = 'Teshi Shell Acceptance'
$trayIcon.Visible = $true
$trayIcon.add_MouseClick({ param($sender,$event)
    Add-Content -LiteralPath (Join-Path $StateDir 'clicks.txt') -Value $event.Button
    if ($event.Button -eq [System.Windows.Forms.MouseButtons]::Right) { $form.Hide() }
})
$trayIcon.add_MouseDoubleClick({ $form.Show(); $form.WindowState = 'Normal' })
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 100
$timer.add_Tick({
    $commandPath = Join-Path $StateDir 'command.txt'
    if (Test-Path -LiteralPath $commandPath) {
        $command = Get-Content -LiteralPath $commandPath -Raw
        Remove-Item -LiteralPath $commandPath
        switch ($command.Trim()) {
            'hide' { $form.Hide() }
            'show' { $form.Show(); $form.WindowState = 'Normal' }
            'minimize' { $form.WindowState = 'Minimized' }
            'recreate' {
                $method = [System.Windows.Forms.Control].GetMethod('RecreateHandle', [System.Reflection.BindingFlags]'Instance,NonPublic')
                $method.Invoke($form, @())
            }
            'quit' { $form.Close() }
        }
    }
})
$timer.Start()
try { [System.Windows.Forms.Application]::Run($form) }
finally { $timer.Dispose(); $trayIcon.Visible = $false; $trayIcon.Dispose(); $form.Dispose() }
