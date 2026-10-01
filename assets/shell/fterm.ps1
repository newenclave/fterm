# fterm shell integration for PowerShell (5.1 and 7).
# It tells fterm the current folder (OSC 7) and when commands start and end (OSC 133).
# fterm loads this file by itself. To turn it off: `shell_integration = false` in fterm.lua.

if ($env:TERM_PROGRAM -ne 'fterm' -or $Global:__FtermLoaded) { return }
$Global:__FtermLoaded = $true
$Global:__FtermLastHistoryId = -1
$Global:__FtermOriginalPrompt = $function:Prompt

function Global:__FtermExitCode {
    if ($? -eq $true) { return 0 }
    $last = Get-History -Count 1
    if ($Error.Count -gt 0 -and $Error[0].InvocationInfo.HistoryId -eq $last.Id) { return 1 }
    if ($null -ne $LASTEXITCODE) { return $LASTEXITCODE }
    return 1
}

function Global:Prompt {
    $code = __FtermExitCode
    $e = [char]27
    $b = [char]7
    $out = ''
    $last = Get-History -Count 1
    if ($Global:__FtermLastHistoryId -ne -1) {
        if ($null -eq $last -or $last.Id -eq $Global:__FtermLastHistoryId) {
            # No new command (for example Enter on an empty line, or Ctrl+C).
            $out += "$e]133;D$b"
        } else {
            $out += "$e]133;D;$code$b"
        }
    }
    $dir = $executionContext.SessionState.Path.CurrentLocation
    if ($dir.Provider.Name -eq 'FileSystem') {
        $path = $dir.ProviderPath.Replace('\', '/')
        $out += "$e]7;file://$env:COMPUTERNAME/$([uri]::EscapeUriString($path))$b"
    }
    $out += "$e]133;A$b"
    $out += $Global:__FtermOriginalPrompt.Invoke()
    $out += "$e]133;B$b"
    $Global:__FtermLastHistoryId = if ($null -ne $last) { $last.Id } else { 0 }
    return $out
}

# The VS Code escape for OSC 633;E: `\` -> `\\`, `;` and control chars -> `\xNN`.
function Global:__FtermEscape([string]$text) {
    $sb = [System.Text.StringBuilder]::new()
    foreach ($c in $text.ToCharArray()) {
        if ($c -eq '\') { [void]$sb.Append('\\') }
        elseif ($c -eq ';' -or [int]$c -lt 32) { [void]$sb.Append('\x{0:x2}' -f [int]$c) }
        else { [void]$sb.Append($c) }
    }
    $sb.ToString()
}

# When Enter is pressed: the command text (OSC 633;E, for the history) and
# OSC 133 C (the command starts), so fterm knows how long a command runs.
if (Get-Module PSReadLine) {
    Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
        $line = $null
        $cursor = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
        $e = [char]27
        $b = [char]7
        if ($line.Trim()) { [Console]::Write("$e]633;E;$(__FtermEscape $line)$b") }
        [Console]::Write("$e]133;C$b")
    }
}
