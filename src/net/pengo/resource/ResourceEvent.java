/*
 * ResourceEvent.java
 *
 * Created on 16 August 2002, 10:07
 */

/**
 *
 * @author  administrator
 */
package net.pengo.resource;

import net.pengo.app.OpenFile;


public class ResourceEvent extends java.util.EventObject {
    OpenFile openFile;
    String category; //FIXME: prolly do this some other way later?
    Resource resource;
    
    /** Creates a new instance of ResourceEvent */
    public ResourceEvent(Object source, OpenFile openFile, String category, Resource resource) {
        super(source);
        this.openFile = openFile;
        this.category = category;
        this.resource = resource;
    }
    
    public String getCategory() {
        return category;
    }
    
    public Resource getResource() {
        return resource;
    }
    
    public OpenFile getOpenFile() {
        return openFile;
    }
    
}
