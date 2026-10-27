#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    [string]$ServiceName = "OnePage",
    [string]$DisplayName = "OnePage",
    [string]$Description = "OnePage team link navigator",
    [string]$ExePath = "",
    [string]$ConfigPath = "",
    [switch]$OpenFirewall,
    [int]$Port = 8686
)

$ErrorActionPreference = "Stop"
$scriptRoot = $PSScriptRoot
$root = Split-Path -Parent $scriptRoot

function Resolve-DefaultPath {
    param(
        [string]$Explicit,
        [string]$FileName,
        [string]$Root
    )
    if ($Explicit) { return (Resolve-Path -LiteralPath $Explicit).Path }
    $candidates = @(
        (Join-Path $Root $FileName),
        (Join-Path $scriptRoot $FileName)
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    throw "Cannot find $FileName. Pass -ExePath or -ConfigPath explicitly."
}

$exe = Resolve-DefaultPath -Explicit $ExePath -FileName "onepage.exe" -Root $root
$config = Resolve-DefaultPath -Explicit $ConfigPath -FileName "onepage.toml" -Root $root

if (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue) {
    throw "Service '$ServiceName' already exists. Run uninstall-service.ps1 first."
}

$binPath = '"' + $exe + '" --service --config "' + $config + '"'
sc.exe create $ServiceName binPath= $binPath start= auto DisplayName= $DisplayName | Out-Null
if ($LASTEXITCODE -ne 0) { throw "sc.exe create failed with exit code $LASTEXITCODE." }

sc.exe description $ServiceName $Description | Out-Null
sc.exe failure $ServiceName reset= 86400 actions= restart/5000/restart/15000/restart/60000 | Out-Null

Start-Service -Name $ServiceName
Start-Sleep -Milliseconds 600

if ($OpenFirewall) {
    $ruleName = "$DisplayName (TCP $Port)"
    if (-not (Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue)) {
        New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -Protocol TCP `
            -LocalPort $Port -Action Allow -Profile Domain,Private | Out-Null
    }
    Write-Host "Firewall rule enabled for TCP port $Port (Domain/Private)."
}

$service = Get-Service -Name $ServiceName
Write-Host "Service : $ServiceName"
Write-Host "Status  : $($service.Status)"
Write-Host "Exe     : $exe"
Write-Host "Config  : $config"
Write-Host "Open    : http://<server-ip>:$Port"
Write-Host "Logs    : see the data_dir configured in onepage.toml (logs\\onepage.log)"
