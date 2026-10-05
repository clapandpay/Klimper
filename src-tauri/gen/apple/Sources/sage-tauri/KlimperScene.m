// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 the Klimper authors
//
// The UIScene life cycle for an app whose window code predates it.
//
// Apps linked against the iOS 27 SDK must adopt UIScene, or UIKit refuses to launch them ("UIScene
// life cycle is required for apps built with this SDK"). tao 0.34, which Sage's pinned Tauri
// uses, creates its UIWindow in application:didFinishLaunchingWithOptions: and listens to the
// application delegate only. tao 0.37 handles scenes itself; until Sage's Tauri gets there, this
// file bridges the two without touching tao:
//
// - Info.ios.plist declares a UIApplicationSceneManifest naming KlimperSceneDelegate.
// - A window shown before any scene exists is held back and attached when the scene connects.
// - Scene events are forwarded to tao's application delegate, which already handles the
//   application-level equivalents (activation, background, URLs, universal links).
//
// Remove this file and the manifest once tao supports scenes (docs/UPSTREAM.md).

#import <UIKit/UIKit.h>
#import <objc/runtime.h>

static NSMutableArray<UIWindow *> *klimperPendingWindows;

static UIWindowScene *KlimperConnectedWindowScene(void) {
    for (UIScene *scene in UIApplication.sharedApplication.connectedScenes) {
        if ([scene isKindOfClass:UIWindowScene.class]) {
            return (UIWindowScene *)scene;
        }
    }
    return nil;
}

@implementation UIWindow (KlimperScene)

+ (void)load {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        Method original = class_getInstanceMethod(UIWindow.class, @selector(makeKeyAndVisible));
        Method replacement =
            class_getInstanceMethod(UIWindow.class, @selector(klimper_makeKeyAndVisible));
        method_exchangeImplementations(original, replacement);
    });
}

- (void)klimper_makeKeyAndVisible {
    if (self.windowScene == nil) {
        UIWindowScene *scene = KlimperConnectedWindowScene();
        if (scene == nil) {
            if (klimperPendingWindows == nil) {
                klimperPendingWindows = [NSMutableArray array];
            }
            if (![klimperPendingWindows containsObject:self]) {
                [klimperPendingWindows addObject:self];
            }
            return;
        }
        self.windowScene = scene;
    }
    // After the exchange this calls UIKit's own makeKeyAndVisible.
    [self klimper_makeKeyAndVisible];
}

@end

@interface KlimperSceneDelegate : UIResponder <UIWindowSceneDelegate>
@property(strong, nonatomic) UIWindow *window;
@end

@implementation KlimperSceneDelegate

static id<UIApplicationDelegate> KlimperAppDelegate(void) {
    return UIApplication.sharedApplication.delegate;
}

- (void)scene:(UIScene *)scene
    willConnectToSession:(UISceneSession *)session
                 options:(UISceneConnectionOptions *)connectionOptions {
    if ([scene isKindOfClass:UIWindowScene.class]) {
        UIWindowScene *windowScene = (UIWindowScene *)scene;
        NSArray<UIWindow *> *pending = [klimperPendingWindows copy];
        [klimperPendingWindows removeAllObjects];
        for (UIWindow *window in pending) {
            window.windowScene = windowScene;
            [window makeKeyAndVisible];
        }
        self.window = pending.firstObject;
    }

    // A cold start from a link arrives here rather than at the application delegate.
    for (UIOpenURLContext *context in connectionOptions.URLContexts) {
        [self forwardURL:context.URL];
    }
    for (NSUserActivity *activity in connectionOptions.userActivities) {
        [self forwardUserActivity:activity];
    }
}

- (void)sceneDidBecomeActive:(UIScene *)scene {
    id<UIApplicationDelegate> delegate = KlimperAppDelegate();
    if ([delegate respondsToSelector:@selector(applicationDidBecomeActive:)]) {
        [delegate applicationDidBecomeActive:UIApplication.sharedApplication];
    }
}

- (void)sceneWillResignActive:(UIScene *)scene {
    id<UIApplicationDelegate> delegate = KlimperAppDelegate();
    if ([delegate respondsToSelector:@selector(applicationWillResignActive:)]) {
        [delegate applicationWillResignActive:UIApplication.sharedApplication];
    }
}

- (void)sceneWillEnterForeground:(UIScene *)scene {
    id<UIApplicationDelegate> delegate = KlimperAppDelegate();
    if ([delegate respondsToSelector:@selector(applicationWillEnterForeground:)]) {
        [delegate applicationWillEnterForeground:UIApplication.sharedApplication];
    }
}

- (void)sceneDidEnterBackground:(UIScene *)scene {
    id<UIApplicationDelegate> delegate = KlimperAppDelegate();
    if ([delegate respondsToSelector:@selector(applicationDidEnterBackground:)]) {
        [delegate applicationDidEnterBackground:UIApplication.sharedApplication];
    }
}

- (void)scene:(UIScene *)scene openURLContexts:(NSSet<UIOpenURLContext *> *)URLContexts {
    for (UIOpenURLContext *context in URLContexts) {
        [self forwardURL:context.URL];
    }
}

- (void)scene:(UIScene *)scene continueUserActivity:(NSUserActivity *)userActivity {
    [self forwardUserActivity:userActivity];
}

- (void)forwardURL:(NSURL *)url {
    id<UIApplicationDelegate> delegate = KlimperAppDelegate();
    if ([delegate respondsToSelector:@selector(application:openURL:options:)]) {
        [delegate application:UIApplication.sharedApplication openURL:url options:@{}];
    }
}

- (void)forwardUserActivity:(NSUserActivity *)activity {
    id<UIApplicationDelegate> delegate = KlimperAppDelegate();
    if ([delegate respondsToSelector:@selector(application:
                                         continueUserActivity:restorationHandler:)]) {
        [delegate application:UIApplication.sharedApplication
            continueUserActivity:activity
              restorationHandler:^(NSArray<id<UIUserActivityRestoring>> *objects){
              }];
    }
}

@end
