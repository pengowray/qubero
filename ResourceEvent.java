/*
 * ResourceEvent.java
 *
 * Created on 16 August 2002, 10:07
 */

/**
 *
 * @author  administrator
 */
class ResourceEvent extends java.util.EventObject {
    String category; //XXX: prolly do this some other way later?
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
