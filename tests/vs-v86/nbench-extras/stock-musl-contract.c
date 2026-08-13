/*
 * Provenance marker for the stock-musl-v1 interpreter audit population.
 * Neither architecture links a benchmark-specific memcpy or memmove
 * replacement. Both binaries resolve those calls from their ordinary static
 * musl target library with compiler builtins disabled.
 *
 * This file is embedded and hashed as the scorecard implementation-source
 * field. It is not compiled or linked into either benchmark executable.
 */
