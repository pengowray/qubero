/**
 * ListResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.util.Collection;
import java.util.List;
import net.pengo.app.OpenFile;

// a resource that has sub resources
public class CollectionResource extends Resource
{
	Collection resourceList;
	public CollectionResource(Collection resourceList, OpenFile openFile) {
		super(openFile);
		this.resourceList = resourceList;
    }
        
        public Collection getCollection() {
            return resourceList;
        }
	
	public Object[] getChildren() {
		return resourceList.toArray();
	}
	
	public String toString() {
		return resourceList.toString();
	}
	
}

