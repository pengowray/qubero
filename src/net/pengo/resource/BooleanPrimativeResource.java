/**
 * BooleanPrimativeResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.dependency.QNode;
import net.pengo.propertyEditor.BooleanPrimativeResourcePropertiesForm;


public class BooleanPrimativeResource  extends BooleanResource
{
    private boolean value;
    
    public BooleanPrimativeResource(boolean value) {
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

