// macOS Security/CommonCrypto boundary for OAuth and JWT verification.
#include <Security/Security.h>
#include <CommonCrypto/CommonDigest.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int field(unsigned char **result, uint32_t *length) {
    unsigned char raw[4];
    if (fread(raw, 1, 4, stdin) != 4) return 0;
    *length = ((uint32_t)raw[0] << 24) | ((uint32_t)raw[1] << 16) | ((uint32_t)raw[2] << 8) | raw[3];
    if (*length > 16 * 1024 * 1024) return 0;
    *result = malloc(*length ? *length : 1);
    if (!*result) return 0;
    return fread(*result, 1, *length, stdin) == *length;
}

static size_t length_size(size_t n) {
    if (n < 128) return 1;
    size_t bytes = 0;
    do { bytes++; n >>= 8; } while (n);
    return 1 + bytes;
}

static unsigned char *length_write(unsigned char *out, size_t n) {
    if (n < 128) { *out++ = (unsigned char)n; return out; }
    size_t bytes = length_size(n) - 1;
    *out++ = (unsigned char)(0x80 | bytes);
    for (size_t i = bytes; i > 0; i--) *out++ = (unsigned char)(n >> (8 * (i - 1)));
    return out;
}

static size_t integer_size(const unsigned char *bytes, size_t n) {
    size_t payload = n + ((bytes[0] & 0x80) ? 1 : 0);
    return 1 + length_size(payload) + payload;
}

static unsigned char *integer_write(unsigned char *out, const unsigned char *bytes, size_t n) {
    size_t payload = n + ((bytes[0] & 0x80) ? 1 : 0);
    *out++ = 0x02;
    out = length_write(out, payload);
    if (payload != n) *out++ = 0;
    memcpy(out, bytes, n);
    return out + n;
}

static int verify(const unsigned char *data, uint32_t data_len,
                  const unsigned char *sig, uint32_t sig_len,
                  const unsigned char *n, uint32_t n_len,
                  const unsigned char *e, uint32_t e_len) {
    size_t inner = integer_size(n, n_len) + integer_size(e, e_len);
    size_t total = 1 + length_size(inner) + inner;
    unsigned char *der = malloc(total);
    if (!der) return 0;
    unsigned char *end = der;
    *end++ = 0x30;
    end = length_write(end, inner);
    end = integer_write(end, n, n_len);
    end = integer_write(end, e, e_len);
    CFDataRef keyData = CFDataCreate(NULL, der, (CFIndex)total);
    free(der);
    if (!keyData) return 0;
    const void *keys[] = { kSecAttrKeyType, kSecAttrKeyClass };
    const void *values[] = { kSecAttrKeyTypeRSA, kSecAttrKeyClassPublic };
    CFDictionaryRef attrs = CFDictionaryCreate(NULL, keys, values, 2,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    SecKeyRef key = SecKeyCreateWithData(keyData, attrs, NULL);
    CFRelease(attrs);
    CFRelease(keyData);
    if (!key) return 0;
    CFDataRef message = CFDataCreate(NULL, data, data_len);
    CFDataRef signature = CFDataCreate(NULL, sig, sig_len);
    int valid = message && signature && SecKeyVerifySignature(key,
        kSecKeyAlgorithmRSASignatureMessagePKCS1v15SHA256, message, signature, NULL);
    if (message) CFRelease(message);
    if (signature) CFRelease(signature);
    CFRelease(key);
    return valid;
}

int main(void) {
    int op = getchar();
    unsigned char *data = NULL, *signature = NULL, *modulus = NULL, *exponent = NULL;
    uint32_t data_len = 0, sig_len = 0, n_len = 0, e_len = 0;
    unsigned char digest[CC_SHA256_DIGEST_LENGTH];
    int ok = 0;
    if (op == 'r') {
        if (!field(&data, &data_len) || data_len != 4) goto done;
        uint32_t count = ((uint32_t)data[0] << 24) | ((uint32_t)data[1] << 16) | ((uint32_t)data[2] << 8) | data[3];
        if (count == 0 || count > 4096) goto done;
        unsigned char *random = malloc(count);
        if (!random) goto done;
        if (SecRandomCopyBytes(kSecRandomDefault, count, random) == errSecSuccess &&
            fwrite(random, 1, count, stdout) == count) ok = 1;
        free(random);
    } else if (op == 'h') {
        if (!field(&data, &data_len)) goto done;
        if (CC_SHA256(data, data_len, digest) && fwrite(digest, 1, sizeof(digest), stdout) == sizeof(digest)) ok = 1;
    } else if (op == 'v') {
        if (!field(&data, &data_len) || !field(&signature, &sig_len) ||
            !field(&modulus, &n_len) || !field(&exponent, &e_len) ||
            !n_len || !e_len || !sig_len) goto done;
        unsigned char valid = verify(data, data_len, signature, sig_len, modulus, n_len, exponent, e_len) ? 1 : 0;
        if (fwrite(&valid, 1, 1, stdout) == 1) ok = 1;
    }
done:
    free(data); free(signature); free(modulus); free(exponent);
    if (!ok) fputs("Native crypto operation failed\n", stderr);
    return ok ? 0 : 1;
}
