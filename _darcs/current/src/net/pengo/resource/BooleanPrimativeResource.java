/**
 * BooleanPrimativeResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

public class BooleanPrimativeResource  extends BooleanResource
{
    private boolean value;
    
    public BooleanPrimativeResource(boolean value) {
        this.value = value;
    }
    
    public Resource[] getSources() {
        // primatives can have no sources.
        return new Resource[]{};
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

    /*
    public void editProperties() {
        //new BooleanPrimativeResourcePropertiesForm(this).show();
        new ResourceForm(this).show();
    }
    */
    
}

