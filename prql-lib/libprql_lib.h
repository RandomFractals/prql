#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Compilation options
 */
typedef struct Options {
  /**
   * Pass generated SQL string trough a formatter that splits it
   * into multiple lines and prettifies indentation and spacing.
   *
   * Defaults to true.
   */
  bool format;
  /**
   * Target and dialect to compile to.
   */
  char *target;
  /**
   * Emits the compiler signature as a comment after generated SQL
   *
   * Defaults to true.
   */
  bool signature_comment;
} Options;

/**
 * Compile a PRQL string into a SQL string.
 *
 * This is a wrapper for: `prql_to_pl`, `pl_to_rq` and `rq_to_sql` without converting to JSON
 * between each of the functions.
 *
 * See `Options` struct for available compilation options.
 */
int compile(const char *prql_query, const struct Options *options, char *out);

/**
 * Build PL AST from a PRQL string
 *
 * Takes PRQL source buffer and writes PL serialized as JSON to `out` buffer.
 *
 * Returns 0 on success and a negative number -1 on failure.
 */
int prql_to_pl(const char *prql_query, char *out);

/**
 * Finds variable references, validates functions calls, determines frames and converts PL to RQ.
 *
 * Takes PL serialized as JSON buffer and writes RQ serialized as JSON to `out` buffer.
 *
 * Returns 0 on success and a negative number -1 on failure.
 */
int pl_to_rq(const char *pl_json, char *out);

/**
 * Convert RQ AST into an SQL string.
 *
 * Takes RQ serialized as JSON buffer and writes SQL source to `out` buffer.
 *
 * Returns 0 on success and a negative number -1 on failure.
 */
int rq_to_sql(const char *rq_json, char *out);
