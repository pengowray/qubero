/**
 * ListResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.util.List;
import net.pengo.app.OpenFile;
import net.pengo.restree.ResourceList;

// a resource that has sub resources
public class ListResource extends Resource
{
	List resourceList;
	public ListResource(List resourceList, OpenFile openFile) {
		super(openFile);
		this.resourceList = resourceList;
    }
	
	public Object[] getChildren() {
		return resourceList.toArray();
	}
	
	public String toString() {
		return resourceList.toString();
	}
	
}

