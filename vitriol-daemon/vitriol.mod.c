#include <linux/module.h>
#include <linux/export-internal.h>
#include <linux/compiler.h>

MODULE_INFO(name, KBUILD_MODNAME);

__visible struct module __this_module
__section(".gnu.linkonce.this_module") = {
	.name = KBUILD_MODNAME,
	.init = init_module,
#ifdef CONFIG_MODULE_UNLOAD
	.exit = cleanup_module,
#endif
	.arch = MODULE_ARCH_INIT,
};



static const struct modversion_info ____versions[]
__used __section("__versions") = {
	{ 0xe5c2991b, "__pci_register_driver" },
	{ 0x4e54d6ac, "cdev_del" },
	{ 0x0bc5fb0d, "unregister_chrdev_region" },
	{ 0x1595e410, "device_destroy" },
	{ 0xa1dacb42, "class_destroy" },
	{ 0x2b6f53b9, "pci_unregister_driver" },
	{ 0x092a35a2, "_copy_to_user" },
	{ 0xe4de56b4, "__ubsan_handle_load_invalid_value" },
	{ 0x91d6d561, "dma_free_attrs" },
	{ 0xabfbaeb0, "pci_iounmap" },
	{ 0x8f1acb38, "pci_release_region" },
	{ 0x201d9cdc, "pci_disable_device" },
	{ 0x2437d1be, "pci_enable_device" },
	{ 0x381ea112, "pci_request_region" },
	{ 0xf145940c, "pci_iomap" },
	{ 0x9ef1423b, "dma_set_mask" },
	{ 0x7039d3ca, "dma_alloc_attrs" },
	{ 0x9ef1423b, "dma_set_coherent_mask" },
	{ 0xd272d446, "__fentry__" },
	{ 0xe8213e80, "_printk" },
	{ 0xd272d446, "__x86_return_thunk" },
	{ 0x9f222e1e, "alloc_chrdev_region" },
	{ 0xd5f66efd, "cdev_init" },
	{ 0x8ea73856, "cdev_add" },
	{ 0x653aa194, "class_create" },
	{ 0xe486c4b7, "device_create" },
	{ 0xbebe66ff, "module_layout" },
};

static const u32 ____version_ext_crcs[]
__used __section("__version_ext_crcs") = {
	0xe5c2991b,
	0x4e54d6ac,
	0x0bc5fb0d,
	0x1595e410,
	0xa1dacb42,
	0x2b6f53b9,
	0x092a35a2,
	0xe4de56b4,
	0x91d6d561,
	0xabfbaeb0,
	0x8f1acb38,
	0x201d9cdc,
	0x2437d1be,
	0x381ea112,
	0xf145940c,
	0x9ef1423b,
	0x7039d3ca,
	0x9ef1423b,
	0xd272d446,
	0xe8213e80,
	0xd272d446,
	0x9f222e1e,
	0xd5f66efd,
	0x8ea73856,
	0x653aa194,
	0xe486c4b7,
	0xbebe66ff,
};
static const char ____version_ext_names[]
__used __section("__version_ext_names") =
	"__pci_register_driver\0"
	"cdev_del\0"
	"unregister_chrdev_region\0"
	"device_destroy\0"
	"class_destroy\0"
	"pci_unregister_driver\0"
	"_copy_to_user\0"
	"__ubsan_handle_load_invalid_value\0"
	"dma_free_attrs\0"
	"pci_iounmap\0"
	"pci_release_region\0"
	"pci_disable_device\0"
	"pci_enable_device\0"
	"pci_request_region\0"
	"pci_iomap\0"
	"dma_set_mask\0"
	"dma_alloc_attrs\0"
	"dma_set_coherent_mask\0"
	"__fentry__\0"
	"_printk\0"
	"__x86_return_thunk\0"
	"alloc_chrdev_region\0"
	"cdev_init\0"
	"cdev_add\0"
	"class_create\0"
	"device_create\0"
	"module_layout\0"
;

MODULE_INFO(depends, "");

MODULE_ALIAS("pci:v000010DEd00001B82sv*sd*bc*sc*i*");

MODULE_INFO(srcversion, "E6B10E3ABF87104957AC988");
