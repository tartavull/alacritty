use std::cell::RefCell;

use objc2::ffi::NSInteger;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{class, define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::NSString;
use winit::event_loop::EventLoopProxy;

use crate::event::{Event, EventType};

const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) << 24 | (b as u32) << 16 | (c as u32) << 8 | (d as u32)
}

const CORE_EVENT_CLASS: u32 = fourcc(b'a', b'e', b'v', b't');
const OPEN_DOCUMENTS_EVENT: u32 = fourcc(b'o', b'd', b'o', b'c');
const KEY_DIRECT_OBJECT: u32 = fourcc(b'-', b'-', b'-', b'-');

struct OpenDocumentsHandlerIvars {
    proxy: EventLoopProxy<Event>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = OpenDocumentsHandlerIvars]
    struct OpenDocumentsHandler;

    impl OpenDocumentsHandler {
        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(&self, event: *mut AnyObject, _reply: *mut AnyObject) {
            let Some(event) = (unsafe { event.as_ref() }) else {
                return;
            };

            let list: *mut AnyObject =
                unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
            if list.is_null() {
                return;
            }

            let count: NSInteger = unsafe { msg_send![list, numberOfItems] };
            if count < 1 {
                return;
            }

            let mut urls = Vec::new();
            for index in 1..=count {
                let item: *mut AnyObject = unsafe { msg_send![list, descriptorAtIndex: index] };
                if item.is_null() {
                    continue;
                }

                let url: *mut AnyObject = unsafe { msg_send![item, fileURLValue] };
                if url.is_null() {
                    continue;
                }

                let path_url: *mut AnyObject = unsafe { msg_send![url, filePathURL] };
                let url = if path_url.is_null() { url } else { path_url };

                let absolute: *mut AnyObject = unsafe { msg_send![url, absoluteString] };
                if absolute.is_null() {
                    continue;
                }

                let url = unsafe { &*(absolute as *const NSString) }.to_string();
                urls.push(url);
            }

            if urls.is_empty() {
                return;
            }

            let _ = self
                .ivars()
                .proxy
                .send_event(Event::new(EventType::OpenUrls(urls), None));
        }
    }
);

thread_local! {
    static OPEN_DOCUMENTS_HANDLER: RefCell<Option<Retained<OpenDocumentsHandler>>> = RefCell::new(None);
}

impl OpenDocumentsHandler {
    fn new(proxy: EventLoopProxy<Event>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = OpenDocumentsHandler::alloc(mtm).set_ivars(OpenDocumentsHandlerIvars { proxy });
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) fn register_open_documents_handler(proxy: EventLoopProxy<Event>) {
    // Register AppleEvent handler to forward opened documents into the event loop.
    let mtm = MainThreadMarker::new().expect("open document handler must be on the main thread");
    let handler = OpenDocumentsHandler::new(proxy, mtm);

    let manager: *mut AnyObject =
        unsafe { msg_send![class!(NSAppleEventManager), sharedAppleEventManager] };
    if manager.is_null() {
        return;
    }

    unsafe {
        let _: () = msg_send![
            manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleOpenDocuments:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: OPEN_DOCUMENTS_EVENT,
        ];
    }

    OPEN_DOCUMENTS_HANDLER.with(|cell| {
        *cell.borrow_mut() = Some(handler);
    });
}
