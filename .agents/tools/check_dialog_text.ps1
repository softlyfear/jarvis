$ErrorActionPreference = 'Stop'
# Exercise the production function with bounded, deterministic UIA range doubles.
# This is a range-algorithm test, not a Windows terminal observation.
Add-Type -TypeDefinition @'
using System;
namespace System.Windows.Automation {
    public enum TextPatternRangeEndpoint { Start, End }
    public enum TextUnit { Character }
    public class TextPattern { public static object Pattern = new object(); }
}
public class TestRange {
    public string Text;
    public int Start, End;
    public TestRange(string text, int start, int end) { Text = text; Start = start; End = end; }
    public TestRange Clone() { return new TestRange(Text, Start, End); }
    int Position(System.Windows.Automation.TextPatternRangeEndpoint endpoint) {
        return endpoint == System.Windows.Automation.TextPatternRangeEndpoint.Start ? Start : End;
    }
    public void MoveEndpointByRange(System.Windows.Automation.TextPatternRangeEndpoint endpoint, TestRange range, System.Windows.Automation.TextPatternRangeEndpoint other) {
        int value = range.Position(other);
        if (endpoint == System.Windows.Automation.TextPatternRangeEndpoint.Start) Start = value; else End = value;
    }
    public int MoveEndpointByUnit(System.Windows.Automation.TextPatternRangeEndpoint endpoint, System.Windows.Automation.TextUnit unit, int count) {
        int before = Start;
        Start = Math.Max(0, Math.Min(Text.Length, Start + count));
        return Start - before;
    }
    public int CompareEndpoints(System.Windows.Automation.TextPatternRangeEndpoint endpoint, TestRange other, System.Windows.Automation.TextPatternRangeEndpoint otherEndpoint) {
        return Position(endpoint).CompareTo(other.Position(otherEndpoint));
    }
    public string GetText(int limit) {
        int length = End - Start;
        return Text.Substring(Start, limit < 0 ? length : Math.Min(limit, length));
    }
}
public class TestPattern {
    public TestRange[] Ranges;
    public TestRange[] GetVisibleRanges() { return Ranges; }
}
public class TestControl {
    public TestPattern Pattern;
    public TestPattern GetCurrentPattern(object pattern) { return Pattern; }
}
'@
$source = Join-Path $PSScriptRoot '../../crates/jarvis-core/src/actions/dialog.ps1'
$tokens = $null; $errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors -join '; ') }
$function = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-ControlText' }, $true)
Invoke-Expression $function.Extent.Text
function Read-Ranges([TestRange[]]$ranges) {
    $control = [TestControl]::new()
    $control.Pattern = [TestPattern]::new()
    $control.Pattern.Ranges = $ranges
    return Get-ControlText $control
}
$text = ('я' * 12000) + "`r`nPS C:\Jarvis> uv init"
$tail = Read-Ranges @([TestRange]::new($text, 0, $text.Length))
if ($tail.Length -ne 8000 -or -not $tail.EndsWith('uv init')) { throw 'Long range lost the typed command' }
$visible = Read-Ranges @([TestRange]::new('hidden visible', 7, 14))
if ($visible -cne 'visible') { throw 'Range extension included hidden scrollback' }
$multiple = Read-Ranges @([TestRange]::new('old', 0, 3), [TestRange]::new('PS> fresh', 0, 9))
if ($multiple -cne "old`nPS> fresh") { throw 'Visible ranges changed order' }
if ((Read-Ranges @()) -cne '') { throw 'Empty ranges should be empty' }
'PASS: long range retains command; hidden text excluded; range order preserved; empty ranges supported'
