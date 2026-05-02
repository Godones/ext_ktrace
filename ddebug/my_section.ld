SECTIONS {
    .dyndbg :
    {
        __start___dyndbg = .;
        KEEP(*(__dyndbg))
        __stop___dyndbg = .;
    }
}
INSERT AFTER .data;
