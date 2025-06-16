print("vm.spawn called")
return {
    l = function()
        --error("This is an error message")
        local table = {}
        for i = 1, 10 do
            print(i)
            table[i] = i * 2
        end

        print("mlua var", _G.test_mluau_var)

        return 13 * 6 + _G.test_mluau_var
    end,
}