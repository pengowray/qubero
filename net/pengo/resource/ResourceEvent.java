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


public class ResourceEvent extends java.util.EventObject {
    String category; //FIXME: prolly do this some other way later?
    Resource resource;
    
    /** Creates a new instance of ResourceEvent */
    public ResourceEvent(Object source, String category, Resource resource) {
        super(source);
        this.category = category;
        this.resource = resource;
    }
    
    public String getCategory() {
        return category;
    }
    
    public Resource getResource() {
        return resource;
    }
    
}
