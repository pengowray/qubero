/**
 * BooleanPrimativeResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.app.OpenFile;
import net.pengo.restree.ResourceList;
import net.pengo.propertyEditor.BooleanPrimativeResourcePropertiesForm;
import net.pengo.dependency.QNode;


public class BooleanPrimativeResource  extends BooleanResource
{
    private boolean value;
    
    public BooleanPrimativeResource(OpenFile of, boolean value) {
        super(of);
        this.value = value;
    }

	public QNode[] getSources() {
		// primatives can have no sources.
		return new QNode[]{};
	}
	
    public boolean isPrimative() {
	return true;
    }
    
    public boolean getValue() {
	return value;
    }
    
    public void setValue(boolean b) {
	value = b;
    }

    public void editProperties() {
	new BooleanPrimativeResourcePropertiesForm(this).show();
    }
    
}

