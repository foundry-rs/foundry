/*******************************************************
 HIDAPI - Multi-Platform library for
 communication with HID devices.

 Alan Ott
 Signal 11 Software

 8/22/2009
 Linux Version - 6/2/2009

 Copyright 2009, All Rights Reserved.

 At the discretion of the user of this library,
 this software may be licensed under the terms of the
 GNU General Public License v3, a BSD-Style license, or the
 original HIDAPI license as outlined in the LICENSE.txt,
 LICENSE-gpl3.txt, LICENSE-bsd.txt, and LICENSE-orig.txt
 files located at the root of the source distribution.
 These files may also be found in the public source
 code repository located at:
        https://github.com/libusb/hidapi .
********************************************************/

/* C */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <locale.h>
#include <errno.h>

/* Unix */
#include <unistd.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/ioctl.h>
#include <sys/utsname.h>
#include <fcntl.h>
#include <poll.h>

/* Linux */
#include <linux/hidraw.h>
#include <linux/version.h>
#include <linux/input.h>
#include <libudev.h>

#include "hidapi.h"

#ifdef HIDAPI_ALLOW_BUILD_WORKAROUND_KERNEL_2_6_39
/* This definitions first appeared in Linux Kernel 2.6.39 in linux/hidraw.h.
    hidapi doesn't support kernels older than that,
    so we don't define macros below explicitly, to fail builds on old kernels.
    For those who really need this as a workaround (e.g. to be able to build on old build machines),
    can workaround by defining the macro above.
*/
#ifndef HIDIOCSFEATURE
#define HIDIOCSFEATURE(len)    _IOC(_IOC_WRITE|_IOC_READ, 'H', 0x06, len)
#endif
#ifndef HIDIOCGFEATURE
#define HIDIOCGFEATURE(len)    _IOC(_IOC_WRITE|_IOC_READ, 'H', 0x07, len)
#endif

#endif


/* USB HID device property names */
const char *device_string_names[] = {
	"manufacturer",
	"product",
	"serial",
};

/* Symbolic names for the properties above */
enum device_string_id {
	DEVICE_STRING_MANUFACTURER,
	DEVICE_STRING_PRODUCT,
	DEVICE_STRING_SERIAL,

	DEVICE_STRING_COUNT,
};

struct hid_device_ {
	int device_handle;
	int blocking;
	int uses_numbered_reports;
	wchar_t *last_error_str;
};

static struct hid_api_version api_version = {
	.major = HID_API_VERSION_MAJOR,
	.minor = HID_API_VERSION_MINOR,
	.patch = HID_API_VERSION_PATCH
};

/* Global error message that is not specific to a device, e.g. for
   hid_open(). It is thread-local like errno. */
__thread wchar_t *last_global_error_str = NULL;

static hid_device *new_hid_device(void)
{
	hid_device *dev = (hid_device*) calloc(1, sizeof(hid_device));
	dev->device_handle = -1;
	dev->blocking = 1;
	dev->uses_numbered_reports = 0;
	dev->last_error_str = NULL;

	return dev;
}


/* The caller must free the returned string with free(). */
static wchar_t *utf8_to_wchar_t(const char *utf8)
{
	wchar_t *ret = NULL;

	if (utf8) {
		size_t wlen = mbstowcs(NULL, utf8, 0);
		if ((size_t) -1 == wlen) {
			return wcsdup(L"");
		}
		ret = (wchar_t*) calloc(wlen+1, sizeof(wchar_t));
		mbstowcs(ret, utf8, wlen+1);
		ret[wlen] = 0x0000;
	}

	return ret;
}


/* Set the last global error to be reported by hid_error(NULL).
 * The given error message will be copied (and decoded according to the
 * currently locale, so do not pass in string constants).
 * The last stored global error message is freed.
 * Use register_global_error(NULL) to indicate "no error". */
static void register_global_error(const char *msg)
{
	if (last_global_error_str)
		free(last_global_error_str);

	last_global_error_str = utf8_to_wchar_t(msg);
}

/* See register_global_error, but you can pass a format string into this function. */
static void register_global_error_format(const char *format, ...)
{
	va_list args;
	va_start(args, format);

	char msg[100];
	vsnprintf(msg, sizeof(msg), format, args);

	va_end(args);

	register_global_error(msg);
}

/* Set the last error for a device to be reported by hid_error(device).
 * The given error message will be copied (and decoded according to the
 * currently locale, so do not pass in string constants).
 * The last stored global error message is freed.
 * Use register_device_error(device, NULL) to indicate "no error". */
static void register_device_error(hid_device *dev, const char *msg)
{
	if (dev->last_error_str)
		free(dev->last_error_str);

	dev->last_error_str = utf8_to_wchar_t(msg);
}

/* See register_device_error, but you can pass a format string into this function. */
static void register_device_error_format(hid_device *dev, const char *format, ...)
{
	va_list args;
	va_start(args, format);

	char msg[100];
	vsnprintf(msg, sizeof(msg), format, args);

	va_end(args);

	register_device_error(dev, msg);
}

/* Get an attribute value from a udev_device and return it as a whar_t
   string. The returned string must be freed with free() when done.*/
static wchar_t *copy_udev_string(struct udev_device *dev, const char *udev_name)
{
	return utf8_to_wchar_t(udev_device_get_sysattr_value(dev, udev_name));
}

/*
 * Gets the size of the HID item at the given position
 * Returns 1 if successful, 0 if an invalid key
 * Sets data_len and key_size when successful
 */
static int get_hid_item_size(__u8 *report_descriptor, unsigned int pos, __u32 size, int *data_len, int *key_size)
{
	int key = report_descriptor[pos];
	int size_code;

	/*
	 * This is a Long Item. The next byte contains the
	 * length of the data section (value) for this key.
	 * See the HID specification, version 1.11, section
	 * 6.2.2.3, titled "Long Items."
	 */
	if ((key & 0xf0) == 0xf0) {
		if (pos + 1 < size)
		{
			*data_len = report_descriptor[pos + 1];
			*key_size = 3;
			return 1;
		}
		*data_len = 0; /* malformed report */
		*key_size = 0;
	}

	/*
	 * This is a Short Item. The bottom two bits of the
	 * key contain the size code for the data section
	 * (value) for this key. Refer to the HID
	 * specification, version 1.11, section 6.2.2.2,
	 * titled "Short Items."
	 */
	size_code = key & 0x3;
	switch (size_code) {
	case 0:
	case 1:
	case 2:
		*data_len = size_code;
		*key_size = 1;
		return 1;
	case 3:
		*data_len = 4;
		*key_size = 1;
		return 1;
	default:
		/* Can't ever happen since size_code is & 0x3 */
		*data_len = 0;
		*key_size = 0;
		break;
	};

	/* malformed report */
	return 0;
}

/* uses_numbered_reports() returns 1 if report_descriptor describes a device
   which contains numbered reports. */
static int uses_numbered_reports(__u8 *report_descriptor, __u32 size) {
	unsigned int i = 0;
	int data_len, key_size;

	while (i < size) {
		int key = report_descriptor[i];

		/* Check for the Report ID key */
		if (key == 0x85/*Report ID*/) {
			/* This device has a Report ID, which means it uses
			   numbered reports. */
			return 1;
		}

		/* Determine data_len and key_size */
		if (!get_hid_item_size(report_descriptor, i, size, &data_len, &key_size))
			return 0; /* malformed report */

		/* Skip over this key and it's associated data */
		i += data_len + key_size;
	}

	/* Didn't find a Report ID key. Device doesn't use numbered reports. */
	return 0;
}

/*
 * Get bytes from a HID Report Descriptor.
 * Only call with a num_bytes of 0, 1, 2, or 4.
 */
static __u32 get_hid_report_bytes(__u8 *rpt, size_t len, size_t num_bytes, size_t cur)
{
	/* Return if there aren't enough bytes. */
	if (cur + num_bytes >= len)
		return 0;

	if (num_bytes == 0)
		return 0;
	else if (num_bytes == 1)
		return rpt[cur + 1];
	else if (num_bytes == 2)
		return (rpt[cur + 2] * 256 + rpt[cur + 1]);
	else if (num_bytes == 4)
		return (
			rpt[cur + 4] * 0x01000000 +
			rpt[cur + 3] * 0x00010000 +
			rpt[cur + 2] * 0x00000100 +
			rpt[cur + 1] * 0x00000001
		);
	else
		return 0;
}

/*
 * Retrieves the device's Usage Page and Usage from the report descriptor.
 * The algorithm returns the current Usage Page/Usage pair whenever a new
 * Collection is found and a Usage Local Item is currently in scope.
 * Usage Local Items are consumed by each Main Item (See. 6.2.2.8).
 * The algorithm should give similar results as Apple's:
 *   https://developer.apple.com/documentation/iokit/kiohiddeviceusagepairskey?language=objc
 * Physical Collections are also matched (macOS does the same).
 *
 * This function can be called repeatedly until it returns non-0
 * Usage is found. pos is the starting point (initially 0) and will be updated
 * to the next search position.
 *
 * The return value is 0 when a pair is found.
 * 1 when finished processing descriptor.
 * -1 on a malformed report.
 */
static int get_next_hid_usage(__u8 *report_descriptor, __u32 size, unsigned int *pos, unsigned short *usage_page, unsigned short *usage)
{
	int data_len, key_size;
	int initial = *pos == 0; /* Used to handle case where no top-level application collection is defined */
	int usage_pair_ready = 0;

	/* Usage is a Local Item, it must be set before each Main Item (Collection) before a pair is returned */
	int usage_found = 0;

	while (*pos < size) {
		int key = report_descriptor[*pos];
		int key_cmd = key & 0xfc;

		/* Determine data_len and key_size */
		if (!get_hid_item_size(report_descriptor, *pos, size, &data_len, &key_size))
			return -1; /* malformed report */

		switch (key_cmd) {
		case 0x4: /* Usage Page 6.2.2.7 (Global) */
			*usage_page = get_hid_report_bytes(report_descriptor, size, data_len, *pos);
			break;

		case 0x8: /* Usage 6.2.2.8 (Local) */
			*usage = get_hid_report_bytes(report_descriptor, size, data_len, *pos);
			usage_found = 1;
			break;

		case 0xa0: /* Collection 6.2.2.4 (Main) */
			/* A Usage Item (Local) must be found for the pair to be valid */
			if (usage_found)
				usage_pair_ready = 1;

			/* Usage is a Local Item, unset it */
			usage_found = 0;
			break;

		case 0x80: /* Input 6.2.2.4 (Main) */
		case 0x90: /* Output 6.2.2.4 (Main) */
		case 0xb0: /* Feature 6.2.2.4 (Main) */
		case 0xc0: /* End Collection 6.2.2.4 (Main) */
			/* Usage is a Local Item, unset it */
			usage_found = 0;
			break;
		}

		/* Skip over this key and it's associated data */
		*pos += data_len + key_size;

		/* Return usage pair */
		if (usage_pair_ready)
			return 0;
	}

	/* If no top-level application collection is found and usage page/usage pair is found, pair is valid
	   https://docs.microsoft.com/en-us/windows-hardware/drivers/hid/top-level-collections */
	if (initial && usage_found)
		return 0; /* success */

	return 1; /* finished processing */
}

/*
 * Retrieves the hidraw report descriptor from a file.
 * When using this form, <sysfs_path>/device/report_descriptor, elevated priviledges are not required.
 */
static int get_hid_report_descriptor(const char *rpt_path, struct hidraw_report_descriptor *rpt_desc)
{
	int rpt_handle;
	ssize_t res;

	rpt_handle = open(rpt_path, O_RDONLY);
	if (rpt_handle < 0) {
		register_global_error_format("open failed (%s): %s", rpt_path, strerror(errno));
		return -1;
	}

	/*
	 * Read in the Report Descriptor
	 * The sysfs file has a maximum size of 4096 (which is the same as HID_MAX_DESCRIPTOR_SIZE) so we should always
	 * be ok when reading the descriptor.
	 * In practice if the HID descriptor is any larger I suspect many other things will break.
	 */
	memset(rpt_desc, 0x0, sizeof(*rpt_desc));
	res = read(rpt_handle, rpt_desc->value, HID_MAX_DESCRIPTOR_SIZE);
	if (res < 0) {
		register_global_error_format("read failed (%s): %s", rpt_path, strerror(errno));
	}
	rpt_desc->size = (__u32) res;

	close(rpt_handle);
	return (int) res;
}

static int get_hid_report_descriptor_from_sysfs(const char *sysfs_path, struct hidraw_report_descriptor *rpt_desc)
{
	int res = -1;
	/* Construct <sysfs_path>/device/report_descriptor */
	size_t rpt_path_len = strlen(sysfs_path) + 25 + 1;
	char* rpt_path = (char*) calloc(1, rpt_path_len);
	snprintf(rpt_path, rpt_path_len, "%s/device/report_descriptor", sysfs_path);

	res = get_hid_report_descriptor(rpt_path, rpt_desc);
	free(rpt_path);

	return res;
}

/*
 * The caller is responsible for free()ing the (newly-allocated) character
 * strings pointed to by serial_number_utf8 and product_name_utf8 after use.
 */
static int
parse_uevent_info(const char *uevent, unsigned *bus_type,
	unsigned short *vendor_id, unsigned short *product_id,
	char **serial_number_utf8, char **product_name_utf8)
{
	char *tmp = strdup(uevent);
	char *saveptr = NULL;
	char *line;
	char *key;
	char *value;

	int found_id = 0;
	int found_serial = 0;
	int found_name = 0;

	line = strtok_r(tmp, "\n", &saveptr);
	while (line != NULL) {
		/* line: "KEY=value" */
		key = line;
		value = strchr(line, '=');
		if (!value) {
			goto next_line;
		}
		*value = '\0';
		value++;

		if (strcmp(key, "HID_ID") == 0) {
			/**
			 *        type vendor   product
			 * HID_ID=0003:000005AC:00008242
			 **/
			int ret = sscanf(value, "%x:%hx:%hx", bus_type, vendor_id, product_id);
			if (ret == 3) {
				found_id = 1;
			}
		} else if (strcmp(key, "HID_NAME") == 0) {
			/* The caller has to free the product name */
			*product_name_utf8 = strdup(value);
			found_name = 1;
		} else if (strcmp(key, "HID_UNIQ") == 0) {
			/* The caller has to free the serial number */
			*serial_number_utf8 = strdup(value);
			found_serial = 1;
		}

next_line:
		line = strtok_r(NULL, "\n", &saveptr);
	}

	free(tmp);
	return (found_id && found_name && found_serial);
}


static int get_device_string(hid_device *dev, enum device_string_id key, wchar_t *string, size_t maxlen)
{
	struct udev *udev;
	struct udev_device *udev_dev, *parent, *hid_dev;
	struct stat s;
	int ret = -1;
	char *serial_number_utf8 = NULL;
	char *product_name_utf8 = NULL;

	/* Create the udev object */
	udev = udev_new();
	if (!udev) {
		register_global_error("Couldn't create udev context");
		return -1;
	}

	/* Get the dev_t (major/minor numbers) from the file handle. */
	ret = fstat(dev->device_handle, &s);
	if (-1 == ret)
		return ret;
	/* Open a udev device from the dev_t. 'c' means character device. */
	udev_dev = udev_device_new_from_devnum(udev, 'c', s.st_rdev);
	if (udev_dev) {
		hid_dev = udev_device_get_parent_with_subsystem_devtype(
			udev_dev,
			"hid",
			NULL);
		if (hid_dev) {
			unsigned short dev_vid;
			unsigned short dev_pid;
			unsigned bus_type;
			size_t retm;

			ret = parse_uevent_info(
			           udev_device_get_sysattr_value(hid_dev, "uevent"),
			           &bus_type,
			           &dev_vid,
			           &dev_pid,
			           &serial_number_utf8,
			           &product_name_utf8);

			/* Standard USB device */
			if (bus_type == BUS_USB) {
				/* This is a USB device. Find its parent USB Device node. */
				parent = udev_device_get_parent_with_subsystem_devtype(
					   udev_dev,
					   "usb",
					   "usb_device");
				if (parent) {
					const char *str;
					const char *key_str = NULL;

					if (key >= 0 && key < DEVICE_STRING_COUNT) {
						key_str = device_string_names[key];
					} else {
						ret = -1;
						goto end;
					}

					str = udev_device_get_sysattr_value(parent, key_str);
					if (str) {
						/* Convert the string from UTF-8 to wchar_t */
						retm = mbstowcs(string, str, maxlen);
						ret = (retm == (size_t)-1)? -1: 0;
					}

					/* USB information parsed */
					goto end;
				}
				else {
					/* Correctly handled below */
				}
			}

			/* USB information not available (uhid) or another type of HID bus */
			switch (bus_type) {
				case BUS_BLUETOOTH:
				case BUS_I2C:
				case BUS_USB:
					switch (key) {
						case DEVICE_STRING_MANUFACTURER:
							wcsncpy(string, L"", maxlen);
							ret = 0;
							break;
						case DEVICE_STRING_PRODUCT:
							retm = mbstowcs(string, product_name_utf8, maxlen);
							ret = (retm == (size_t)-1)? -1: 0;
							break;
						case DEVICE_STRING_SERIAL:
							retm = mbstowcs(string, serial_number_utf8, maxlen);
							ret = (retm == (size_t)-1)? -1: 0;
							break;
						case DEVICE_STRING_COUNT:
						default:
							ret = -1;
							break;
					}
			}
		}
	}

end:
	free(serial_number_utf8);
	free(product_name_utf8);

	udev_device_unref(udev_dev);
	/* parent and hid_dev don't need to be (and can't be) unref'd.
	   I'm not sure why, but they'll throw double-free() errors. */
	udev_unref(udev);

	return ret;
}

HID_API_EXPORT const struct hid_api_version* HID_API_CALL hid_version()
{
	return &api_version;
}

HID_API_EXPORT const char* HID_API_CALL hid_version_str()
{
	return HID_API_VERSION_STR;
}

int HID_API_EXPORT hid_init(void)
{
	const char *locale;

	/* Set the locale if it's not set. */
	locale = setlocale(LC_CTYPE, NULL);
	if (!locale)
		setlocale(LC_CTYPE, "");

	return 0;
}

int HID_API_EXPORT hid_exit(void)
{
	/* Free global error message */
	register_global_error(NULL);

	return 0;
}


struct hid_device_info  HID_API_EXPORT *hid_enumerate(unsigned short vendor_id, unsigned short product_id)
{
	struct udev *udev;
	struct udev_enumerate *enumerate;
	struct udev_list_entry *devices, *dev_list_entry;

	struct hid_device_info *root = NULL; /* return object */
	struct hid_device_info *cur_dev = NULL;
	struct hid_device_info *prev_dev = NULL; /* previous device */

	hid_init();

	/* Create the udev object */
	udev = udev_new();
	if (!udev) {
		register_global_error("Couldn't create udev context");
		return NULL;
	}

	/* Create a list of the devices in the 'hidraw' subsystem. */
	enumerate = udev_enumerate_new(udev);
	udev_enumerate_add_match_subsystem(enumerate, "hidraw");
	udev_enumerate_scan_devices(enumerate);
	devices = udev_enumerate_get_list_entry(enumerate);
	/* For each item, see if it matches the vid/pid, and if so
	   create a udev_device record for it */
	udev_list_entry_foreach(dev_list_entry, devices) {
		const char *sysfs_path;
		const char *dev_path;
		const char *str;
		struct udev_device *raw_dev; /* The device's hidraw udev node. */
		struct udev_device *hid_dev; /* The device's HID udev node. */
		struct udev_device *usb_dev; /* The device's USB udev node. */
		struct udev_device *intf_dev; /* The device's interface (in the USB sense). */
		unsigned short dev_vid;
		unsigned short dev_pid;
		char *serial_number_utf8 = NULL;
		char *product_name_utf8 = NULL;
		unsigned bus_type;
		int result;
		struct hidraw_report_descriptor report_desc;

		/* Get the filename of the /sys entry for the device
		   and create a udev_device object (dev) representing it */
		sysfs_path = udev_list_entry_get_name(dev_list_entry);
		raw_dev = udev_device_new_from_syspath(udev, sysfs_path);
		dev_path = udev_device_get_devnode(raw_dev);

		hid_dev = udev_device_get_parent_with_subsystem_devtype(
			raw_dev,
			"hid",
			NULL);

		if (!hid_dev) {
			/* Unable to find parent hid device. */
			goto next;
		}

		result = parse_uevent_info(
			udev_device_get_sysattr_value(hid_dev, "uevent"),
			&bus_type,
			&dev_vid,
			&dev_pid,
			&serial_number_utf8,
			&product_name_utf8);

		if (!result) {
			/* parse_uevent_info() failed for at least one field. */
			goto next;
		}

		/* Filter out unhandled devices right away */
		switch (bus_type) {
			case BUS_BLUETOOTH:
			case BUS_I2C:
			case BUS_USB:
				break;

			default:
				goto next;
		}

		/* Check the VID/PID against the arguments */
		if ((vendor_id == 0x0 || vendor_id == dev_vid) &&
		    (product_id == 0x0 || product_id == dev_pid)) {
			struct hid_device_info *tmp;

			/* VID/PID match. Create the record. */
			tmp = (struct hid_device_info*) calloc(1, sizeof(struct hid_device_info));
			if (cur_dev) {
				cur_dev->next = tmp;
			}
			else {
				root = tmp;
			}
			prev_dev = cur_dev;
			cur_dev = tmp;

			/* Fill out the record */
			cur_dev->next = NULL;
			cur_dev->path = dev_path? strdup(dev_path): NULL;

			/* VID/PID */
			cur_dev->vendor_id = dev_vid;
			cur_dev->product_id = dev_pid;

			/* Serial Number */
			cur_dev->serial_number = utf8_to_wchar_t(serial_number_utf8);

			/* Release Number */
			cur_dev->release_number = 0x0;

			/* Interface Number */
			cur_dev->interface_number = -1;

			switch (bus_type) {
				case BUS_USB:
					/* The device pointed to by raw_dev contains information about
					   the hidraw device. In order to get information about the
					   USB device, get the parent device with the
					   subsystem/devtype pair of "usb"/"usb_device". This will
					   be several levels up the tree, but the function will find
					   it. */
					usb_dev = udev_device_get_parent_with_subsystem_devtype(
							raw_dev,
							"usb",
							"usb_device");

					/* uhid USB devices
					   Since this is a virtual hid interface, no USB information will
					   be available. */
					if (!usb_dev) {
						/* Manufacturer and Product strings */
						cur_dev->manufacturer_string = wcsdup(L"");
						cur_dev->product_string = utf8_to_wchar_t(product_name_utf8);
						break;
					}

					/* Manufacturer and Product strings */
					cur_dev->manufacturer_string = copy_udev_string(usb_dev, device_string_names[DEVICE_STRING_MANUFACTURER]);
					cur_dev->product_string = copy_udev_string(usb_dev, device_string_names[DEVICE_STRING_PRODUCT]);

					/* Release Number */
					str = udev_device_get_sysattr_value(usb_dev, "bcdDevice");
					cur_dev->release_number = (str)? strtol(str, NULL, 16): 0x0;

					/* Get a handle to the interface's udev node. */
					intf_dev = udev_device_get_parent_with_subsystem_devtype(
							raw_dev,
							"usb",
							"usb_interface");
					if (intf_dev) {
						str = udev_device_get_sysattr_value(intf_dev, "bInterfaceNumber");
						cur_dev->interface_number = (str)? strtol(str, NULL, 16): -1;
					}

					break;

				case BUS_BLUETOOTH:
				case BUS_I2C:
					/* Manufacturer and Product strings */
					cur_dev->manufacturer_string = wcsdup(L"");
					cur_dev->product_string = utf8_to_wchar_t(product_name_utf8);

					break;

				default:
					/* Unknown device type - this should never happen, as we
					 * check for USB and Bluetooth devices above */
					break;
			}

			/* Usage Page and Usage */
			result = get_hid_report_descriptor_from_sysfs(sysfs_path, &report_desc);
			if (result >= 0) {
				unsigned short page = 0, usage = 0;
				unsigned int pos = 0;
				/*
				 * Parse the first usage and usage page
				 * out of the report descriptor.
				 */
				if (!get_next_hid_usage(report_desc.value, report_desc.size, &pos, &page, &usage)) {
					cur_dev->usage_page = page;
					cur_dev->usage = usage;
				}

				/*
				 * Parse any additional usage and usage pages
				 * out of the report descriptor.
				 */
				while (!get_next_hid_usage(report_desc.value, report_desc.size, &pos, &page, &usage)) {
					/* Create new record for additional usage pairs */
					tmp = (struct hid_device_info*) calloc(1, sizeof(struct hid_device_info));
					cur_dev->next = tmp;
					prev_dev = cur_dev;
					cur_dev = tmp;

					/* Update fields */
					cur_dev->path = strdup(dev_path);
					cur_dev->vendor_id = dev_vid;
					cur_dev->product_id = dev_pid;
					cur_dev->serial_number = prev_dev->serial_number? wcsdup(prev_dev->serial_number): NULL;
					cur_dev->release_number = prev_dev->release_number;
					cur_dev->interface_number = prev_dev->interface_number;
					cur_dev->manufacturer_string = prev_dev->manufacturer_string? wcsdup(prev_dev->manufacturer_string): NULL;
					cur_dev->product_string = prev_dev->product_string? wcsdup(prev_dev->product_string): NULL;
					cur_dev->usage_page = page;
					cur_dev->usage = usage;
				}
			}
		}

	next:
		free(serial_number_utf8);
		free(product_name_utf8);
		udev_device_unref(raw_dev);
		/* hid_dev, usb_dev and intf_dev don't need to be (and can't be)
		   unref()d.  It will cause a double-free() error.  I'm not
		   sure why.  */
	}
	/* Free the enumerator and udev objects. */
	udev_enumerate_unref(enumerate);
	udev_unref(udev);

	return root;
}

void  HID_API_EXPORT hid_free_enumeration(struct hid_device_info *devs)
{
	struct hid_device_info *d = devs;
	while (d) {
		struct hid_device_info *next = d->next;
		free(d->path);
		free(d->serial_number);
		free(d->manufacturer_string);
		free(d->product_string);
		free(d);
		d = next;
	}
}

hid_device * hid_open(unsigned short vendor_id, unsigned short product_id, const wchar_t *serial_number)
{
	/* Set global error to none */
	register_global_error(NULL);

	struct hid_device_info *devs, *cur_dev;
	const char *path_to_open = NULL;
	hid_device *handle = NULL;

	devs = hid_enumerate(vendor_id, product_id);
	cur_dev = devs;
	while (cur_dev) {
		if (cur_dev->vendor_id == vendor_id &&
		    cur_dev->product_id == product_id) {
			if (serial_number) {
				if (wcscmp(serial_number, cur_dev->serial_number) == 0) {
					path_to_open = cur_dev->path;
					break;
				}
			}
			else {
				path_to_open = cur_dev->path;
				break;
			}
		}
		cur_dev = cur_dev->next;
	}

	if (path_to_open) {
		/* Open the device */
		handle = hid_open_path(path_to_open);
	} else {
		register_global_error("No such device");
	}

	hid_free_enumeration(devs);

	return handle;
}

hid_device * HID_API_EXPORT hid_open_path(const char *path)
{
	/* Set global error to none */
	register_global_error(NULL);

	hid_device *dev = NULL;

	hid_init();

	dev = new_hid_device();

	/* OPEN HERE */
	dev->device_handle = open(path, O_RDWR);

	/* If we have a good handle, return it. */
	if (dev->device_handle >= 0) {
		/* Set device error to none */
		register_device_error(dev, NULL);

		/* Get the report descriptor */
		int res, desc_size = 0;
		struct hidraw_report_descriptor rpt_desc;

		memset(&rpt_desc, 0x0, sizeof(rpt_desc));

		/* Get Report Descriptor Size */
		res = ioctl(dev->device_handle, HIDIOCGRDESCSIZE, &desc_size);
		if (res < 0)
			register_device_error_format(dev, "ioctl (GRDESCSIZE): %s", strerror(errno));

		/* Get Report Descriptor */
		rpt_desc.size = desc_size;
		res = ioctl(dev->device_handle, HIDIOCGRDESC, &rpt_desc);
		if (res < 0) {
			register_device_error_format(dev, "ioctl (GRDESC): %s", strerror(errno));
		} else {
			/* Determine if this device uses numbered reports. */
			dev->uses_numbered_reports =
				uses_numbered_reports(rpt_desc.value,
				                      rpt_desc.size);
		}

		return dev;
	}
	else {
		/* Unable to open any devices. */
		register_global_error(strerror(errno));
		free(dev);
		return NULL;
	}
}


int HID_API_EXPORT hid_write(hid_device *dev, const unsigned char *data, size_t length)
{
	int bytes_written;

	if (!data || (length == 0)) {
		errno = EINVAL;
		register_device_error(dev, strerror(errno));
		return -1;
	}

	bytes_written = write(dev->device_handle, data, length);

	register_device_error(dev, (bytes_written == -1)? strerror(errno): NULL);

	return bytes_written;
}


int HID_API_EXPORT hid_read_timeout(hid_device *dev, unsigned char *data, size_t length, int milliseconds)
{
	/* Set device error to none */
	register_device_error(dev, NULL);

	int bytes_read;

	if (milliseconds >= 0) {
		/* Milliseconds is either 0 (non-blocking) or > 0 (contains
		   a valid timeout). In both cases we want to call poll()
		   and wait for data to arrive.  Don't rely on non-blocking
		   operation (O_NONBLOCK) since some kernels don't seem to
		   properly report device disconnection through read() when
		   in non-blocking mode.  */
		int ret;
		struct pollfd fds;

		fds.fd = dev->device_handle;
		fds.events = POLLIN;
		fds.revents = 0;
		ret = poll(&fds, 1, milliseconds);
		if (ret == 0) {
			/* Timeout */
			return ret;
		}
		if (ret == -1) {
			/* Error */
			register_device_error(dev, strerror(errno));
			return ret;
		}
		else {
			/* Check for errors on the file descriptor. This will
			   indicate a device disconnection. */
			if (fds.revents & (POLLERR | POLLHUP | POLLNVAL))
				// We cannot use strerror() here as no -1 was returned from poll().
				return -1;
		}
	}

	bytes_read = read(dev->device_handle, data, length);
	if (bytes_read < 0) {
		if (errno == EAGAIN || errno == EINPROGRESS)
			bytes_read = 0;
		else
			register_device_error(dev, strerror(errno));
	}

	return bytes_read;
}

int HID_API_EXPORT hid_read(hid_device *dev, unsigned char *data, size_t length)
{
	return hid_read_timeout(dev, data, length, (dev->blocking)? -1: 0);
}

int HID_API_EXPORT hid_set_nonblocking(hid_device *dev, int nonblock)
{
	/* Do all non-blocking in userspace using poll(), since it looks
	   like there's a bug in the kernel in some versions where
	   read() will not return -1 on disconnection of the USB device */

	dev->blocking = !nonblock;
	return 0; /* Success */
}


int HID_API_EXPORT hid_send_feature_report(hid_device *dev, const unsigned char *data, size_t length)
{
	int res;

	res = ioctl(dev->device_handle, HIDIOCSFEATURE(length), data);
	if (res < 0)
		register_device_error_format(dev, "ioctl (SFEATURE): %s", strerror(errno));

	return res;
}

int HID_API_EXPORT hid_get_feature_report(hid_device *dev, unsigned char *data, size_t length)
{
	int res;

	res = ioctl(dev->device_handle, HIDIOCGFEATURE(length), data);
	if (res < 0)
		register_device_error_format(dev, "ioctl (GFEATURE): %s", strerror(errno));

	return res;
}

// Not supported by Linux HidRaw yet
int HID_API_EXPORT HID_API_CALL hid_get_input_report(hid_device *dev, unsigned char *data, size_t length)
{
	(void)dev;
	(void)data;
	(void)length;
	return -1;
}

void HID_API_EXPORT hid_close(hid_device *dev)
{
	if (!dev)
		return;

	int ret = close(dev->device_handle);

	register_global_error((ret == -1)? strerror(errno): NULL);

	/* Free the device error message */
	register_device_error(dev, NULL);

	free(dev);
}


int HID_API_EXPORT_CALL hid_get_manufacturer_string(hid_device *dev, wchar_t *string, size_t maxlen)
{
	return get_device_string(dev, DEVICE_STRING_MANUFACTURER, string, maxlen);
}

int HID_API_EXPORT_CALL hid_get_product_string(hid_device *dev, wchar_t *string, size_t maxlen)
{
	return get_device_string(dev, DEVICE_STRING_PRODUCT, string, maxlen);
}

int HID_API_EXPORT_CALL hid_get_serial_number_string(hid_device *dev, wchar_t *string, size_t maxlen)
{
	return get_device_string(dev, DEVICE_STRING_SERIAL, string, maxlen);
}

int HID_API_EXPORT_CALL hid_get_indexed_string(hid_device *dev, int string_index, wchar_t *string, size_t maxlen)
{
	(void)dev;
	(void)string_index;
	(void)string;
	(void)maxlen;
	return -1;
}


/* Passing in NULL means asking for the last global error message. */
HID_API_EXPORT const wchar_t * HID_API_CALL  hid_error(hid_device *dev)
{
	if (dev) {
		if (dev->last_error_str == NULL)
			return L"Success";
		return dev->last_error_str;
	}

	if (last_global_error_str == NULL)
		return L"Success";
	return last_global_error_str;
}
