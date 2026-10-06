# Captured iterator integration on resizable-buffer source

Recorded candidate base ffaf603d4dd1f3f3f93948e6a94f579b10e7e9b0.

The eager iterator ticket changes the old binary_collect_source call to use its captured method. The newer resizable-buffer implementation already replaces that helper: binary_source_method reads and validates once, then binary_iterator_values calls that captured method once and consumes a captured direct iterator record. Retain this complete newer implementation, including array-like allocation/write order and resize-sensitive factory behavior. Apply the eager iterator ticket to its other shared consumers and retain all its controls. No legacy binary_collect_source is reintroduced.

Qualification is pending on the complete combined source; neither historical lane proof is relabeled as this source.
