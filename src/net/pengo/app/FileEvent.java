/*
 * ClipboardEvent.java
 *
 * Created on 16 August 2002, 18:17
 */

/**
 *
 * @author  administrator
 */
package net.pengo.app;

public class FileEvent extends java.util.EventObject {
    protected OpenFile openFile;
    
    public FileEvent(Object source, OpenFile openFile) {
        super(source);
        this.openFile = openFile;
    }
    
    public OpenFile getOpenFile() {
        return openFile;
    }
    
}
