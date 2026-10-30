// Linux OS libcrypto boundary for OAuth randomness, hashing and RSA verification.
// Input: op byte, then big-endian u32 length + bytes per field.
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

// Opaque OpenSSL ABI declarations keep the build independent of development
// headers; the OS-supplied libcrypto is loaded only when this helper runs.
typedef struct bignum_st BIGNUM;
typedef struct rsa_st RSA;
static int (*native_rand)(unsigned char *, int);
static unsigned char *(*native_sha256)(const unsigned char *, size_t, unsigned char *);
static BIGNUM *(*native_bn)(const unsigned char *, int, BIGNUM *);
static void (*native_bn_free)(BIGNUM *);
static RSA *(*native_rsa_new)(void);
static void (*native_rsa_free)(RSA *);
static int (*native_set_key)(RSA *, BIGNUM *, BIGNUM *, BIGNUM *);
static int (*native_verify)(int, const unsigned char *, unsigned int, const unsigned char *, unsigned int, RSA *);
static int (*native_nid)(const char *);
#define SHA256_DIGEST_LENGTH 32

static int load_crypto(void) {
    void *library = dlopen("libcrypto.so.3", RTLD_NOW | RTLD_LOCAL);
    if (!library) library = dlopen("libcrypto.so.1.1", RTLD_NOW | RTLD_LOCAL);
    if (!library) return 0;
#define SYMBOL(slot, name) do { *(void **)(&slot) = dlsym(library, name); if (!slot) return 0; } while (0)
    SYMBOL(native_rand, "RAND_bytes");
    SYMBOL(native_sha256, "SHA256");
    SYMBOL(native_bn, "BN_bin2bn");
    SYMBOL(native_bn_free, "BN_free");
    SYMBOL(native_rsa_new, "RSA_new");
    SYMBOL(native_rsa_free, "RSA_free");
    SYMBOL(native_set_key, "RSA_set0_key");
    SYMBOL(native_verify, "RSA_verify");
    SYMBOL(native_nid, "OBJ_txt2nid");
#undef SYMBOL
    return 1;
}

static int field(unsigned char **result, uint32_t *length) {
    unsigned char raw[4];
    if (fread(raw, 1, 4, stdin) != 4) return 0;
    *length = ((uint32_t)raw[0] << 24) | ((uint32_t)raw[1] << 16) | ((uint32_t)raw[2] << 8) | raw[3];
    if (*length > 16 * 1024 * 1024) return 0;
    *result = malloc(*length ? *length : 1);
    if (!*result) return 0;
    return fread(*result, 1, *length, stdin) == *length;
}

int main(void) {
    if (!load_crypto()) { fputs("OS libcrypto unavailable\n", stderr); return 1; }
    int op = getchar();
    unsigned char *data = NULL, *signature = NULL, *modulus = NULL, *exponent = NULL;
    uint32_t data_len = 0, sig_len = 0, n_len = 0, e_len = 0;
    unsigned char digest[SHA256_DIGEST_LENGTH];
    int ok = 0;
    if (op == 'r') {
        if (!field(&data, &data_len) || data_len != 4) goto done;
        uint32_t count = ((uint32_t)data[0] << 24) | ((uint32_t)data[1] << 16) | ((uint32_t)data[2] << 8) | data[3];
        if (count == 0 || count > 4096) goto done;
        unsigned char *random = malloc(count);
        if (!random) goto done;
        if (native_rand(random, (int)count) == 1 && fwrite(random, 1, count, stdout) == count) ok = 1;
        free(random);
    } else if (op == 'h') {
        if (!field(&data, &data_len)) goto done;
        if (native_sha256(data, data_len, digest) && fwrite(digest, 1, sizeof(digest), stdout) == sizeof(digest)) ok = 1;
    } else if (op == 'v') {
        if (!field(&data, &data_len) || !field(&signature, &sig_len) ||
            !field(&modulus, &n_len) || !field(&exponent, &e_len) ||
            n_len == 0 || e_len == 0 || sig_len == 0) goto done;
        if (!native_sha256(data, data_len, digest)) goto done;
        BIGNUM *n = native_bn(modulus, (int)n_len, NULL);
        BIGNUM *e = native_bn(exponent, (int)e_len, NULL);
        RSA *key = native_rsa_new();
        if (!n || !e || !key) { native_bn_free(n); native_bn_free(e); native_rsa_free(key); goto done; }
        if (native_set_key(key, n, e, NULL) != 1) { native_bn_free(n); native_bn_free(e); native_rsa_free(key); goto done; }
        unsigned char valid = native_verify(native_nid("SHA256"), digest, sizeof(digest), signature, sig_len, key) == 1 ? 1 : 0;
        native_rsa_free(key);
        if (fwrite(&valid, 1, 1, stdout) == 1) ok = 1;
    }
done:
    free(data);
    free(signature);
    free(modulus);
    free(exponent);
    if (!ok) fputs("Native crypto operation failed\n", stderr);
    return ok ? 0 : 1;
}
