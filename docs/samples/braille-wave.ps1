# Draws a sine wave with Braille chars. Each cell has 2x4 dots.
# Run: powershell -File docs/samples/braille-wave.ps1
param([int]$Cols = 60, [int]$Rows = 8)
$bits = @(@(0x01, 0x02, 0x04, 0x40), @(0x08, 0x10, 0x20, 0x80))
$w = $Cols * 2
$h = $Rows * 4
$cells = New-Object 'int[,]' $Rows, $Cols
for ($x = 0; $x -lt $w; $x++) {
    $y = [int][math]::Round(($h - 1) / 2 * (1 - [math]::Sin($x / $w * 4 * [math]::PI)))
    # Fill down to the middle line, so the wave is a solid shape.
    $mid = [int](($h - 1) / 2)
    $from = [math]::Min($y, $mid)
    $to = [math]::Max($y, $mid)
    for ($yy = $from; $yy -le $to; $yy++) {
        $cells[[int][math]::Floor($yy / 4), [int][math]::Floor($x / 2)] =
            $cells[[int][math]::Floor($yy / 4), [int][math]::Floor($x / 2)] -bor $bits[$x % 2][$yy % 4]
    }
}
for ($r = 0; $r -lt $Rows; $r++) {
    $line = ""
    for ($c = 0; $c -lt $Cols; $c++) { $line += [char](0x2800 + $cells[$r, $c]) }
    Write-Host $line
}
