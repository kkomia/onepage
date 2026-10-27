#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    [string]$ServiceName = "OnePage",
    [string]$DisplayName = "OnePage",
    [switch]$RemoveFirewall
)

$ErrorActionPreference = "Stop"

$service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if (-not $service) {
    Write-Host "Service '$ServiceName' is not installed."
} else {
    if ($service.Status -ne "Stopped") {
        Stop-Service -Name $ServiceName -Force
        $service.WaitForStatus("Stopped", [TimeSpan]::FromSeconds(20))
    }
    sc.exe delete $ServiceName | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "sc.exe delete failed with exit code $LASTEXITCODE." }
    Write-Host "Service '$ServiceName' removed."
    Write-Host "Application data and logs were left untouched."
}

if ($RemoveFirewall) {
    $rules = Get-NetFirewallRule -DisplayName "$DisplayName*" -ErrorAction SilentlyContinue
    foreach ($rule in $rules) {
        Remove-NetFirewallRule -Name $rule.Name
    }
    if ($rules) { Write-Host "Firewall rules removed." }
}
