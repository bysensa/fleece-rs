// A helper over the delta functions of couchbase/fleece, for the cross-check.
//   delta create OLD.json NEW.json   -> the JSON delta
//   delta apply  OLD.json DELTA.json -> the new value as JSON
#include "fleece/Fleece.h"
#include "fleece/FLExpert.h"
#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>

static std::string slurp(const char* path) {
    std::ifstream file(path, std::ios::binary);
    std::stringstream text;
    text << file.rdbuf();
    return text.str();
}

int main(int argc, char** argv) {
    if (argc != 4) return 1;
    std::string mode = argv[1], a = slurp(argv[2]), b = slurp(argv[3]);
    FLError error;
    FLDoc old = FLDoc_FromJSON(FLSlice{a.data(), a.size()}, &error);
    if (!old) return 2;
    if (mode == "create") {
        FLDoc now = FLDoc_FromJSON(FLSlice{b.data(), b.size()}, &error);
        if (!now) return 2;
        FLSliceResult delta = FLCreateJSONDelta(FLDoc_GetRoot(old), FLDoc_GetRoot(now));
        fwrite(delta.buf, 1, delta.size, stdout);
    } else {
        FLSliceResult result =
            FLApplyJSONDelta(FLDoc_GetRoot(old), FLSlice{b.data(), b.size()}, &error);
        if (!result.buf) {
            fprintf(stderr, "apply failed: error %d\n", (int)error);
            return 3;
        }
        FLDoc doc = FLDoc_FromResultData(result, kFLTrusted, nullptr, kFLSliceNull);
        FLStringResult json = FLValue_ToJSON(FLDoc_GetRoot(doc));
        fwrite(json.buf, 1, json.size, stdout);
    }
    return 0;
}
