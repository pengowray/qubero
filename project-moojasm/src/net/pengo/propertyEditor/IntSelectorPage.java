/**
 * IntSelectorPage.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;

import net.pengo.resource.IntAddressedResource;

public class IntSelectorPage extends EditablePage
{
    private IntAddressedResource  res;
    
    public IntSelectorPage(IntAddressedResource res, AbstractResourcePropertiesForm form) {
	super(form);
	this.res = res;
	
    }
    
    protected void saveOp() {
	// TODO
    }
    
    public void buildOp() {
	// TODO
    }
    
    public String toString() {
	return "Integer resource selector";
    }
}

