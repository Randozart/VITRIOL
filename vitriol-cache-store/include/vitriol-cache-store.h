#ifndef VITRIOL_CACHE_STORE_H
#define VITRIOL_CACHE_STORE_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct vitriol_cache_handle vitriol_cache_handle;

/* JSON control messages only. No KV payload crosses this boundary. */
vitriol_cache_handle * vitriol_cache_open(const char * config_json, char ** error_out);
char * vitriol_cache_call(vitriol_cache_handle * handle, const char * command_json);
void vitriol_cache_close(vitriol_cache_handle * handle);
void vitriol_cache_string_free(char * value);

#ifdef __cplusplus
}
#endif

#endif
