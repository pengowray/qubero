
package net.pengo.propertyEditor;


import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;
/**
 *
 * @author  Peter Halasz
 *
 */
public class BooleanAddressedResourcePropertiesForm extends AbstractResourcePropertiesForm {
    private BooleanAddressedResource boolRes;
    
    /** Creates a new instance of BooleanResourcePropertiesForm */
    public BooleanAddressedResourcePropertiesForm(BooleanAddressedResource boolRes) {
        super();
        this.boolRes = boolRes;
    }
    
    protected PropertyPage[] getMenu() {
        return new PropertyPage[] {
	    new SetBooleanPage(boolRes, this),
		new AddressPage(boolRes, this),
		new BooleanRbitPage(boolRes, this),
		new QNodePage(boolRes),
        };
    }
    
}
