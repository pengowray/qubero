/*
 * ClipboardEvent.java
 *
 * Created on 16 August 2002, 18:17
 */

/**
 *
 * @author  administrator
 */
package net.pengo.data;

import net.pengo.app.OpenFile;

public class DataEvent extends java.util.EventObject {
    DiffData.Mod e;
    
    /** Creates a new instance of ClipboardEvent */
    public DataEvent(Object source, DiffData.Mod e) {
        super(source);
	this.e = e;
    }
    
    //fixme: DiffData.Mod needs to be less inner
    //public getMod
    
}
