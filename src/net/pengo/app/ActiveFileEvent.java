/*
 * ActiveFileEvent.java
 *
 * Created on July 13, 2003, 9:51 PM
 */

package net.pengo.app;

import java.util.EventObject;
/**
 *
 * @author  Smiley
 */
public class ActiveFileEvent extends EventObject {
    
    /** Creates a new instance of ActiveFileEvent */
    public ActiveFileEvent(Object source, ActiveFile activeFile, OpenFile openFile) {
        super(source);
    }
    
}
