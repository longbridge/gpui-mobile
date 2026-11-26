#include <stdbool.h>
#include <stddef.h>
void *regression_start(void);
void regression_frame(void);
void regression_set_document(const char *text, size_t caret);
char *regression_snapshot(void);
void regression_free(char *text);
void regression_hide_keyboard(void);
void regression_set_editable_range(size_t start, size_t end, bool reports_length);
