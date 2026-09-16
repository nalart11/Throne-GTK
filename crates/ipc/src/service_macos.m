#import <Foundation/Foundation.h>
#import <ServiceManagement/ServiceManagement.h>

static NSString *const ThroneHelperPlist = @"dev.nalart.ThroneGtk.helper.plist";

long throne_helper_status(void) {
    if (@available(macOS 13.0, *)) {
        return [SMAppService daemonServiceWithPlistName:ThroneHelperPlist].status;
    }
    return -1;
}

bool throne_helper_register(char *errorBuffer, size_t errorBufferSize) {
    if (@available(macOS 13.0, *)) {
        NSError *error = nil;
        BOOL result = [[SMAppService daemonServiceWithPlistName:ThroneHelperPlist]
            registerAndReturnError:&error];
        if (!result && errorBuffer != NULL && errorBufferSize > 0) {
            const char *message = error.localizedDescription.UTF8String;
            snprintf(errorBuffer, errorBufferSize, "%s", message != NULL ? message : "unknown error");
        }
        return result;
    }
    if (errorBuffer != NULL && errorBufferSize > 0) {
        snprintf(errorBuffer, errorBufferSize, "%s", "SMAppService requires macOS 13 or newer");
    }
    return false;
}

void throne_helper_open_settings(void) {
    if (@available(macOS 13.0, *)) {
        [SMAppService openSystemSettingsLoginItems];
    }
}
