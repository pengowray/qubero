/*
 * Created on Apr 9, 2004
 *
 */
package net.pengo.propertyEditor;

import net.pengo.resource.Resource;

/**
 * A MultiPage defined by the Resource it is based on.
 *
 * Allows cursor to be restored (on a rebuilt menu/list).
 *
 * @author Peter Halasz
 *
 */
public class ResourceMultiPage extends MultiPage {
    
    private Resource resource;
    
    /**
     * @param form
     * @param page
     * @param name
     */
    public ResourceMultiPage(PropertiesForm form, Resource res) {
        super(form, (PropertyPage[]) res.getPrimaryPages().toArray(new PropertyPage[0]), res.nameOrType());
        this.resource = res;
        // add a listener to the "nameOrType"
    }
    
    /**
     * Getter for property resource.
     * @return Value of property resource.
     */
    public Resource getResource() {
        return resource;
    }
    
    /**
     * Setter for property resource.
     * @param resource New value of property resource.
     */
    public void setResource(Resource resource) {
        //FIXME: (re)set pages and name
        this.resource = resource;
    }
   
    
}
