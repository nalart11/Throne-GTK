#import <Foundation/Foundation.h>
#import <ServiceManagement/ServiceManagement.h>
#include <unistd.h>
#include "../../../core/helpertrust/security.h"

static NSString *const NolurixProtectedBundle = @"/Library/Application Support/Nolurix/Nolurix.app";

// Private implementation names come from the shared security header, not the
// public helper ABI. Keep caller/definition names synchronized.
bool nolurix_helper_safe_acl(const char *path) { return throne_safe_acl(path) == 0; }
bool nolurix_helper_signature(int pid) {
    return throne_signature("/Library/Application Support/Nolurix/Nolurix.app/Contents/MacOS/nolurix-core", pid) == 0;
}

static NSString *const NolurixHelperPlist = @"dev.nalart.Nolurix.helper.plist";

// Run in the logged-in user's context, before replacing/removing the bundle.
// The command-line caller need not start GTK or a dispatch main loop.
bool nolurix_helper_unregister(char *errorBuffer, size_t errorBufferSize) {
    if (getuid() == 0 || geteuid() == 0) {
        if (errorBuffer && errorBufferSize)
            snprintf(errorBuffer, errorBufferSize, "%s", "Run --unregister-helper as the logged-in user, without sudo");
        return false;
    }
    if (@available(macOS 13.0, *)) {
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block BOOL success = NO;
        __block NSString *message = nil;
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
            @autoreleasepool {
                NSError *error = nil;
                SMAppService *service = [SMAppService daemonServiceWithPlistName:NolurixHelperPlist];
                success = service.status == SMAppServiceStatusNotRegistered ||
                    [service unregisterAndReturnError:&error];
                message = error.localizedDescription;
                dispatch_semaphore_signal(done);
            }
        });
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC)) != 0) {
            if (errorBuffer && errorBufferSize)
                snprintf(errorBuffer, errorBufferSize, "%s", "Service Management unregister timed out; check helper status before replacing the app");
            return false;
        }
        if (!success && errorBuffer && errorBufferSize)
            snprintf(errorBuffer, errorBufferSize, "%s", message.UTF8String ?: "Service Management unregister failed");
        return success;
    }
    if (errorBuffer && errorBufferSize)
        snprintf(errorBuffer, errorBufferSize, "%s", "SMAppService requires macOS 13 or newer");
    return false;
}

long nolurix_helper_status(void) {
    if (@available(macOS 13.0, *)) {
        return [SMAppService daemonServiceWithPlistName:NolurixHelperPlist].status;
    }
    return -1;
}

bool nolurix_helper_register(char *errorBuffer, size_t errorBufferSize) {
    if (![[[NSBundle mainBundle].bundlePath stringByResolvingSymlinksInPath] isEqualToString:NolurixProtectedBundle]) {
        if (errorBuffer != NULL && errorBufferSize > 0)
            snprintf(errorBuffer, errorBufferSize, "%s", "Install the PKG and launch the protected installed Nolurix app before registering the helper");
        return false;
    }
    if (@available(macOS 13.0, *)) {
        NSError *error = nil;
        BOOL result = [[SMAppService daemonServiceWithPlistName:NolurixHelperPlist]
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

void nolurix_helper_open_settings(void) {
    if (@available(macOS 13.0, *)) {
        [SMAppService openSystemSettingsLoginItems];
    }
}
