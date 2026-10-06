# Whole flexible-size CSS producer and computed shorthand

Source only, UNQUALIFIED. Recorded published base 5efe03ecd.
Primary specification read2026-10-06: https://www.w3.org/TR/css-flexbox-1/#flex-property and https://www.w3.org/TR/css-flexbox-1/#flex-components.
Existing own parser/math/typed unit machinery is reused; no copied upstream algorithm, dependency or Cargo change.

The authentic e66 literal parsing compact has20 flex failures (14 computed,6 invalid). A declaration parser and values::parses now share one atomic grammar; variables/attributes defer every longhand through the existing cascade protocol. Finite grow/shrink math computes and clamps to nonnegative numbers before the existing layout string consumer. The computed shorthand always derives from winning longhands, including live public setter changes and clones. Basis computation uses actual style font/root units, preserves percentages and box-dependent math for existing used sizing, and retains CSS sizing keyword/basis handoff.

Original nonfinite factor raw/Unparsed assertions and the existing omitted-basis0% compatibility policy stay intact. Layout's broader nonfinite and contextual intrinsic-size behavior is not newly implemented or claimed complete. The literal runner's fixture-font/container geometry limitation stays honest; no hardcoded40px, expected-derived actual or case promotion. Twelve whole new groups retain atomic invalid precedence, component order, math/ranges, actual font/root context, percentage handoff, intrinsic keywords, deferred winner failure, CSS-wide/layer cascade, setter/clone behavior and original nonfinite controls. All original assertions remain unchanged.

Containment/aspect whole20 qualifications completed independently; no prerequisites are copied into this source. Freeze current source, own full CSS17 + full8/2421/945 parsing/render controls and original stable baseline hashes before one supported native gate with actual capacity admission. Root merged-head workspace/docs/full16 remains authoritative.

Before CSS17 proof is the original e66 native parsing0 and qualified stable identity comparison0, preserved outer1; entire Amazing CSS source subtree is byteexact published5efe before this ticket. Latest root E651 full8 native0/fetch0 compacts have the same current published engine/Cargo tree, pin/profile/render settings; original AFTER role/source bytes remain intact. Fresh AFTER whole native tests will compare against these authentic before inputs without a redundant before build or role relabel.

Original782 failed allfour compilation phases101 with actualouter1: E0308fullthree-component arms inferred&&strarrays. Source successor fixes every component reference explicitly and types the shared arm result[&str;3]; all12groups/originalassertions untouched. Public bounded diagnostic17/17native controls, actualmission0/fetch0 binds original compiler cluster; no unchangedsource rerun or testwaiver.


The 8736f9a0 qualification retained a genuine nextest exit100 (5273/2/17skip)
and one lost official case, `flex-basis:calc(0% + 10px)`. The successor computes
font dimensions through the existing length normalizer while preserving percentage
type and the actual container handoff; no corpus expectation changed. The newly
authored inheritance fixture now uses `var(--wide, inherit)` because CSS-wide
custom-property declarations default the custom property itself. The original
no-fallback input remains a separate guaranteed-invalid/unset control. Its original
expected inherited value is retained for the valid fallback fixture.
Specification: https://www.w3.org/TR/css-variables-1/#using-variables
