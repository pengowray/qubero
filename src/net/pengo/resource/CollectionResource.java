/**
 * ListResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.util.Collection;

// a resource that has sub resources
public class CollectionResource extends Resource
{
    Collection resourceList;
    
    public CollectionResource(Collection resourceList) {
        this.resourceList = resourceList;
    }
        
    public Collection getCollection() {
        return resourceList;
    }

//    public String toString() {
//        return resourceList.toString();
//    }
	
    public String valueDesc() {
        return resourceList.size() + " item(s)";
    }
    
    public String getName() {
        if (super.getName() == null) {
            return resourceList.toString();
        }
        
        return super.getName();
    }
    
    public boolean isPointer() {
        return true;
    }


    public Resource[] getSources() {
        return new Resource[]{};
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        
    }
}

